use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use clap::ValueEnum;

use crate::index::ProjectIndex;
use crate::pathing::{canonical_utf8, resolve_in_root};
use crate::scanner;
use crate::snapshot;
use crate::symbols::{Symbol, SymbolKind};
use crate::usage::{self, UsageInput};

// TODO: make configurable via .repolensrc.toml [read] compact_max_function_lines
const COMPACT_MAX_FUNCTION_LINES: usize = 20;
// TODO: make configurable via .repolensrc.toml [read] aggressive_max_output_lines
const AGGRESSIVE_MAX_OUTPUT_LINES: usize = 120;

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum ReadLevel {
    Normal,
    Compact,
    Aggressive,
}

pub fn read(
    root: &Path,
    path: &Utf8Path,
    lines: Option<&str>,
    level: ReadLevel,
    max_bytes: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<()> {
    let output = read_text_with_level(root, path, lines, level, max_bytes, expected_hash)?;
    usage::log_usage(
        &output.root,
        UsageInput {
            cmd: "read",
            file: Some(&output.rel_path),
            level: Some(level.as_str()),
            parser: output.parser.as_deref(),
            fallback: Some(output.fallback),
            bytes_raw: output.bytes_raw,
            bytes_out: output.content.len(),
        },
    );
    let content = output.content;
    print!("{content}");
    Ok(())
}

pub fn read_text(
    root: &Path,
    path: &Utf8Path,
    lines: Option<&str>,
    max_bytes: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<String> {
    Ok(read_text_with_level(
        root,
        path,
        lines,
        ReadLevel::Normal,
        max_bytes,
        expected_hash,
    )?
    .content)
}

pub struct ReadOutput {
    pub root: camino::Utf8PathBuf,
    pub rel_path: camino::Utf8PathBuf,
    pub content: String,
    pub bytes_raw: usize,
    pub parser: Option<String>,
    pub fallback: bool,
}

pub fn read_text_with_level(
    root: &Path,
    path: &Utf8Path,
    lines: Option<&str>,
    level: ReadLevel,
    max_bytes: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<ReadOutput> {
    let root = canonical_utf8(root)?;
    let (target, rel_path) = resolve_in_root(&root, path)?;
    let bytes = fs::read(&target).with_context(|| format!("failed to read {target}"))?;
    if scanner::looks_binary(&bytes) {
        bail!("repolens cannot read binary file: {rel_path}");
    }
    let bytes_raw = bytes.len();
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if let Some(expected_hash) = expected_hash
        && actual_hash != expected_hash
    {
        bail!("hash mismatch");
    }
    let bytes = match max_bytes {
        Some(max_bytes) => &bytes[..bytes.len().min(max_bytes)],
        None => &bytes,
    };
    let content = String::from_utf8_lossy(bytes);
    let (content, parser, fallback) = match level {
        ReadLevel::Normal => (content.to_string(), None, false),
        ReadLevel::Compact | ReadLevel::Aggressive => {
            compact_content(&root, &rel_path, &actual_hash, &content, level)
        }
    };
    let content = format_lines(&content, lines)?;
    Ok(ReadOutput {
        root,
        rel_path,
        content,
        bytes_raw,
        parser,
        fallback,
    })
}

impl ReadLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Compact => "compact",
            Self::Aggressive => "aggressive",
        }
    }
}

fn compact_content(
    root: &camino::Utf8Path,
    rel_path: &camino::Utf8Path,
    actual_hash: &str,
    content: &str,
    level: ReadLevel,
) -> (String, Option<String>, bool) {
    let Some(parser) = parser_name(rel_path) else {
        return (line_based(content, level), None, true);
    };

    let Ok(index) = load_current_index(root, rel_path, actual_hash) else {
        return (line_based(content, level), None, true);
    };
    let mut symbols = index
        .symbols
        .iter()
        .filter(|symbol| symbol.path == rel_path)
        .cloned()
        .collect::<Vec<_>>();
    symbols.sort_by_key(|symbol| (symbol.line, symbol.end_line));

    match level {
        ReadLevel::Normal => (content.to_string(), None, false),
        ReadLevel::Compact => (compact_with_symbols(content, &symbols), Some(parser), false),
        ReadLevel::Aggressive => (
            aggressive_with_symbols(content, &symbols),
            Some(parser),
            false,
        ),
    }
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

fn compact_with_symbols(content: &str, symbols: &[Symbol]) -> String {
    let mut out = String::new();
    let mut skip_until = 0;
    let mut omissions = symbols
        .iter()
        .filter(|symbol| {
            matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                && symbol.end_line.saturating_sub(symbol.line) + 1 > COMPACT_MAX_FUNCTION_LINES
        })
        .map(|symbol| (symbol.line, symbol.end_line))
        .collect::<Vec<_>>();
    omissions.sort_unstable();

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no <= skip_until {
            continue;
        }
        if let Some((_, end)) = omissions.iter().find(|(start, _)| *start == line_no) {
            out.push_str(line);
            out.push('\n');
            let omitted = end.saturating_sub(line_no);
            out.push_str(&format!(
                "{}// ... body omitted ({omitted} lines)\n",
                leading_indent(line)
            ));
            skip_until = *end;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn aggressive_with_symbols(content: &str, symbols: &[Symbol]) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_import_or_module_line(line) {
            selected.push(idx + 1);
        }
    }
    for symbol in symbols {
        if matches!(
            symbol.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Interface
                | SymbolKind::Trait
                | SymbolKind::Impl
                | SymbolKind::Type
                | SymbolKind::Const
                | SymbolKind::Module
        ) {
            selected.push(symbol.line);
        }
    }
    selected.sort_unstable();
    selected.dedup();

    let mut out = String::new();
    for line_no in selected.into_iter().take(AGGRESSIVE_MAX_OUTPUT_LINES) {
        if let Some(line) = lines.get(line_no.saturating_sub(1)) {
            out.push_str(trim_body(line));
            out.push('\n');
        }
    }
    if out.lines().count() >= AGGRESSIVE_MAX_OUTPUT_LINES {
        out.push_str("// ... output truncated (aggressive limit: 120 lines)\n");
    }
    out
}

fn line_based(content: &str, level: ReadLevel) -> String {
    let mut out = String::new();
    match level {
        ReadLevel::Normal => content.to_string(),
        ReadLevel::Compact => {
            out.push_str(
                "// RepoLens: language not recognized; using line-based compact fallback.\n",
            );
            let mut last_written = 0;
            for (idx, line) in content.lines().enumerate() {
                let line_no = idx + 1;
                if line_no <= 40 || is_structural_line(line) {
                    if last_written + 1 < line_no {
                        out.push_str("// ... lines omitted\n");
                    }
                    out.push_str(line);
                    out.push('\n');
                    last_written = line_no;
                }
            }
            out
        }
        ReadLevel::Aggressive => {
            out.push_str(
                "// RepoLens: language not recognized; using line-based aggressive fallback.\n",
            );
            for line in content
                .lines()
                .filter(|line| is_structural_line(line))
                .take(AGGRESSIVE_MAX_OUTPUT_LINES)
            {
                out.push_str(trim_body(line));
                out.push('\n');
            }
            out
        }
    }
}

fn parser_name(path: &camino::Utf8Path) -> Option<String> {
    match path.extension()? {
        "rs" => Some("tree-sitter-rust".to_string()),
        "ts" | "mts" | "cts" => Some("tree-sitter-typescript".to_string()),
        "tsx" => Some("tree-sitter-tsx".to_string()),
        "js" | "mjs" | "cjs" | "jsx" => Some("tree-sitter-javascript".to_string()),
        "py" | "pyw" => Some("tree-sitter-python".to_string()),
        _ => None,
    }
}

fn is_structural_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    is_import_or_module_line(trimmed)
        || [
            "fn ",
            "pub fn ",
            "def ",
            "class ",
            "struct ",
            "enum ",
            "interface ",
            "type ",
            "impl ",
            "trait ",
            "function ",
            "export function ",
        ]
        .iter()
        .any(|keyword| trimmed.starts_with(keyword))
}

fn is_import_or_module_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    [
        "use ", "mod ", "import ", "from ", "include ", "#include", "require", "package ",
        "module ",
    ]
    .iter()
    .any(|keyword| trimmed.starts_with(keyword))
}

fn trim_body(line: &str) -> &str {
    if is_import_or_module_line(line) {
        return line;
    }
    line.split_once('{')
        .map(|(head, _)| head.trim_end())
        .unwrap_or(line)
}

fn leading_indent(line: &str) -> &str {
    let len = line.len() - line.trim_start().len();
    &line[..len]
}

fn format_lines(content: &str, range: Option<&str>) -> Result<String> {
    let (start, end) = match range {
        Some(raw) => parse_line_range(raw)?,
        None => (1, usize::MAX),
    };

    let mut out = String::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no >= start && line_no <= end {
            out.push_str(&format!("{line_no}: {line}\n"));
        }
        if line_no > end {
            break;
        }
    }
    Ok(out)
}

fn parse_line_range(raw: &str) -> Result<(usize, usize)> {
    let Some((start, end)) = raw.split_once('-') else {
        let line = raw.parse::<usize>().context("invalid line number")?;
        return Ok((line, line));
    };
    let start = start.parse::<usize>().context("invalid range start")?;
    let end = end.parse::<usize>().context("invalid range end")?;
    if start == 0 || end < start {
        bail!("invalid line range");
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use camino::{Utf8Path, Utf8PathBuf};

    use super::{ReadLevel, parse_line_range, read_text, read_text_with_level};

    #[test]
    fn parses_line_ranges() {
        assert_eq!(parse_line_range("2-4").unwrap(), (2, 4));
        assert_eq!(parse_line_range("3").unwrap(), (3, 3));
        assert!(parse_line_range("4-2").is_err());
    }

    #[test]
    fn reads_absolute_path_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let file = root.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let file = Utf8PathBuf::from_path_buf(file).unwrap();
        let out = read_text(&root, &file, Some("1"), None, None).unwrap();

        assert_eq!(out, "1: fn main() {}\n");
    }

    #[test]
    fn rejects_absolute_path_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(root_dir.path()).unwrap();
        let outside = outside_dir.path().join("secret.txt");
        std::fs::write(&outside, "secret\n").unwrap();

        let outside = Utf8PathBuf::from_path_buf(dunce::canonicalize(outside).unwrap()).unwrap();

        assert!(read_text(&root, &outside, None, None, None).is_err());
    }

    #[test]
    fn compact_level_omits_long_function_body() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let body = (0..25)
            .map(|idx| format!("    let value_{idx} = {idx};"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            root.join("main.rs"),
            format!("fn large() {{\n{body}\n}}\nfn small() {{}}\n"),
        )
        .unwrap();

        let out = read_text_with_level(
            &root,
            Utf8Path::new("main.rs"),
            None,
            ReadLevel::Compact,
            None,
            None,
        )
        .unwrap();

        assert!(out.content.contains("body omitted"));
        assert!(out.content.contains("fn small()"));
    }

    #[test]
    fn aggressive_level_keeps_signatures() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(
            root.join("main.rs"),
            "use anyhow::Result;\n\npub struct App;\n\nfn run() -> Result<()> {\n    println!(\"run\");\n    Ok(())\n}\n",
        )
        .unwrap();

        let out = read_text_with_level(
            &root,
            Utf8Path::new("main.rs"),
            None,
            ReadLevel::Aggressive,
            None,
            None,
        )
        .unwrap();

        assert!(out.content.contains("use anyhow::Result;"));
        assert!(out.content.contains("pub struct App"));
        assert!(out.content.contains("fn run() -> Result<()>"));
        assert!(!out.content.contains("println!"));
    }
}
