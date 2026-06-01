use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use serde::Serialize;

use crate::deps;
use crate::index::ProjectIndex;
use crate::pathing::{canonical_utf8, resolve_in_root};
use crate::scanner;
use crate::snapshot;
use crate::symbols::{self, SymbolKind};
use crate::usage::{self, UsageInput};

#[derive(Debug, Serialize, Default)]
struct SymbolCounts {
    functions: usize,
    structs: usize,
    enums: usize,
    traits: usize,
    impls: usize,
    classes: usize,
    interfaces: usize,
}

#[derive(Debug, Serialize)]
struct SmartReport {
    file: String,
    language: String,
    parser: Option<String>,
    fallback: bool,
    size_bytes: usize,
    symbols: SymbolCounts,
    imports: Vec<String>,
    summary: Vec<String>,
}

pub fn print(root: &Path, path: &Utf8Path) -> Result<()> {
    let root = canonical_utf8(root)?;
    let (target, rel_path) = resolve_in_root(&root, path)?;
    let bytes = fs::read(&target).with_context(|| format!("failed to read {target}"))?;
    if scanner::looks_binary(&bytes) {
        bail!("repolens cannot read binary file: {rel_path}");
    }

    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    let index = load_current_index(&root, &rel_path, &actual_hash)?;
    let report = build_report(&index, &rel_path, bytes.len());
    let output = serde_json::to_string_pretty(&report)?;
    usage::log_usage(
        &root,
        UsageInput {
            cmd: "smart",
            file: Some(&rel_path),
            level: None,
            parser: report.parser.as_deref(),
            fallback: Some(report.fallback),
            bytes_raw: bytes.len(),
            bytes_out: output.len(),
        },
    );
    println!("{output}");
    Ok(())
}

fn load_current_index(
    root: &camino::Utf8Path,
    rel_path: &camino::Utf8Path,
    actual_hash: &str,
) -> Result<ProjectIndex> {
    let index = snapshot::load_or_build(root.as_std_path())?;
    if index
        .file_by_path(&rel_path.to_path_buf())
        .is_some_and(|file| file.hash == actual_hash)
    {
        return Ok(index);
    }
    ProjectIndex::build(root.as_std_path())
}

fn build_report(
    index: &crate::index::ProjectIndex,
    path: &camino::Utf8Path,
    size_bytes: usize,
) -> SmartReport {
    let path_buf = path.to_path_buf();
    let file_symbols = symbols::outline(index, &path_buf);
    let mut counts = SymbolCounts::default();
    for symbol in &file_symbols {
        match symbol.kind {
            SymbolKind::Function | SymbolKind::Method => counts.functions += 1,
            SymbolKind::Struct => counts.structs += 1,
            SymbolKind::Enum => counts.enums += 1,
            SymbolKind::Trait => counts.traits += 1,
            SymbolKind::Impl => counts.impls += 1,
            SymbolKind::Class => counts.classes += 1,
            SymbolKind::Interface => counts.interfaces += 1,
            _ => {}
        }
    }

    let imports = deps::deps_for_file(index, &path_buf)
        .into_iter()
        .flat_map(|file_deps| file_deps.imports.into_iter().map(|import| import.module))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    SmartReport {
        file: path.as_str().replace('\\', "/"),
        language: language_name(path).to_string(),
        parser: parser_name(path),
        fallback: parser_name(path).is_none(),
        size_bytes,
        summary: summary(&file_symbols, &imports, size_bytes),
        symbols: counts,
        imports,
    }
}

fn language_name(path: &camino::Utf8Path) -> &'static str {
    match path.extension() {
        Some("rs") => "Rust",
        Some("ts") | Some("mts") | Some("cts") => "TypeScript",
        Some("tsx") => "TSX",
        Some("js") | Some("mjs") | Some("cjs") => "JavaScript",
        Some("jsx") => "JSX",
        Some("py") | Some("pyw") => "Python",
        Some("go") => "Go",
        Some("php") => "PHP",
        Some("java") => "Java",
        Some("cs") => "C#",
        Some("c") | Some("h") => "C",
        Some("cc") | Some("cpp") | Some("cxx") | Some("hpp") | Some("hh") | Some("hxx") => "C++",
        Some("rb") => "Ruby",
        Some("json") => "JSON",
        Some("toml") => "TOML",
        Some("yml") | Some("yaml") => "YAML",
        _ => "Unknown",
    }
}

fn parser_name(path: &camino::Utf8Path) -> Option<String> {
    match path.extension()? {
        "rs" => Some("tree-sitter-rust".to_string()),
        "ts" | "mts" | "cts" => Some("tree-sitter-typescript".to_string()),
        "tsx" => Some("tree-sitter-tsx".to_string()),
        "js" | "mjs" | "cjs" | "jsx" => Some("tree-sitter-javascript".to_string()),
        "py" | "pyw" => Some("tree-sitter-python".to_string()),
        "go" => Some("tree-sitter-go".to_string()),
        "php" => Some("tree-sitter-php".to_string()),
        "java" => Some("tree-sitter-java".to_string()),
        "cs" => Some("tree-sitter-c-sharp".to_string()),
        "c" | "h" => Some("tree-sitter-c".to_string()),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some("tree-sitter-cpp".to_string()),
        "rb" => Some("tree-sitter-ruby".to_string()),
        _ => None,
    }
}

fn summary(
    symbols: &[crate::symbols::Symbol],
    imports: &[String],
    size_bytes: usize,
) -> Vec<String> {
    // HEURISTIC: summary is generated from symbol names and import list,
    // not from semantic analysis. Do not present as AI-generated.
    let mut summary = Vec::new();
    if symbols.iter().any(|symbol| symbol.name == "main") {
        summary.push("Application entrypoint.".to_string());
    }
    if imports
        .iter()
        .any(|module| matches!(module.as_str(), "clap" | "axum" | "express" | "django"))
    {
        summary.push("Uses a known CLI or web framework.".to_string());
    }
    if size_bytes > 20_000 {
        summary.push("Large file; consider splitting.".to_string());
    }
    summary
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use crate::index::ProjectIndex;

    use super::build_report;

    #[test]
    fn smart_report_counts_symbols_and_imports() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            "use clap::Parser;\n\npub struct Cli;\n\nfn main() {}\n",
        )
        .unwrap();
        let index = ProjectIndex::build(dir.path()).unwrap();

        let report = build_report(&index, Utf8Path::new("main.rs"), 52);

        assert_eq!(report.file, "main.rs");
        assert_eq!(report.language, "Rust");
        assert_eq!(report.parser.as_deref(), Some("tree-sitter-rust"));
        assert!(!report.fallback);
        assert_eq!(report.symbols.functions, 1);
        assert_eq!(report.symbols.structs, 1);
        assert!(report.imports.contains(&"clap::Parser".to_string()));
        assert!(
            report
                .summary
                .contains(&"Application entrypoint.".to_string())
        );
    }
}
