use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use clap::ValueEnum;

use crate::config::Config;
use crate::index::ProjectIndex;
use crate::pathing::{canonical_utf8, resolve_in_root};
use crate::scanner;
use crate::security;
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
            savings_source: Some(savings_source(level, lines, max_bytes)),
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
    if !Config::load(root.as_std_path())?.allow_sensitive
        && security::is_sensitive_path(target.as_std_path())
    {
        bail!("refusing to read sensitive path");
    }
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
        ReadLevel::Normal => (source_lines(&content), None, false),
        ReadLevel::Compact | ReadLevel::Aggressive => {
            compact_content(&root, &rel_path, &actual_hash, &content, level)
        }
    };
    let content = if level == ReadLevel::Aggressive {
        let range = lines.map(parse_line_range).transpose()?;
        let mut selected = content
            .into_iter()
            .filter(|(line, _)| match (line, range) {
                (Some(line), Some((start, end))) => *line >= start && *line <= end,
                _ => true,
            })
            .collect::<Vec<_>>();
        let count = selected.iter().filter(|(line, _)| line.is_some()).count();
        let mut kept = 0;
        selected.retain(|(line, _)| {
            if line.is_some() {
                kept += 1;
            }
            kept <= AGGRESSIVE_MAX_OUTPUT_LINES
        });
        if count > AGGRESSIVE_MAX_OUTPUT_LINES {
            selected.push((
                None,
                "// ... output truncated (aggressive limit: 120 lines)".to_owned(),
            ));
        }
        format_lines(&selected, None)?
    } else {
        format_lines(&content, lines)?
    };
    Ok(ReadOutput {
        root,
        rel_path,
        content,
        bytes_raw,
        parser,
        fallback,
    })
}

fn savings_source(level: ReadLevel, lines: Option<&str>, max_bytes: Option<usize>) -> &'static str {
    match level {
        ReadLevel::Compact => "compact",
        ReadLevel::Aggressive => "aggressive",
        ReadLevel::Normal => match (lines.is_some(), max_bytes.is_some()) {
            (true, true) => "line_range_and_byte_cap",
            (true, false) => "line_range",
            (false, true) => "byte_cap",
            (false, false) => "full_read",
        },
    }
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
) -> (Vec<ReadLine>, Option<String>, bool) {
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
        ReadLevel::Normal => (source_lines(content), None, false),
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
    let mut index = snapshot::load_or_build(root.as_std_path())?;
    if index
        .file_by_path(&rel_path.to_path_buf())
        .is_some_and(|file| file.hash == actual_hash)
    {
        return Ok(index);
    }
    index.analyze_explicit_file(&rel_path.to_path_buf())?;
    Ok(index)
}

// Generated annotations have no source coordinate and must never shift source lines.
type ReadLine = (Option<usize>, String);

fn source_lines(content: &str) -> Vec<ReadLine> {
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| (Some(idx + 1), line.to_string()))
        .collect()
}

fn compact_with_symbols(content: &str, symbols: &[Symbol]) -> Vec<ReadLine> {
    let mut out = Vec::new();
    let mut skip_until = 0;
    let mut omissions = symbols
        .iter()
        .filter(|symbol| {
            matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                && symbol.end_line.saturating_sub(symbol.line) + 1 > COMPACT_MAX_FUNCTION_LINES
        })
        .filter_map(|symbol| {
            symbol
                .signature_end_line
                .map(|header| (symbol.line, header, symbol.end_line))
        })
        .collect::<Vec<_>>();
    omissions.sort_unstable();

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no <= skip_until {
            continue;
        }
        if let Some((_, header, end)) = omissions.iter().find(|(start, _, _)| *start == line_no) {
            for (offset, header_line) in content
                .lines()
                .enumerate()
                .skip(idx)
                .take(header.saturating_sub(line_no) + 1)
            {
                out.push((Some(offset + 1), header_line.to_owned()));
            }
            let omitted = end.saturating_sub(*header);
            out.push((
                None,
                format!(
                    "{}// ... body omitted ({omitted} lines)",
                    leading_indent(line)
                ),
            ));
            skip_until = *end;
            continue;
        }
        out.push((Some(line_no), line.to_string()));
    }
    out
}

fn aggressive_with_symbols(content: &str, symbols: &[Symbol]) -> Vec<ReadLine> {
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

    let mut out = Vec::new();
    for line_no in selected {
        if let Some(line) = lines.get(line_no.saturating_sub(1)) {
            out.push((Some(line_no), trim_body(line).to_string()));
        }
    }
    out
}

fn line_based(content: &str, level: ReadLevel) -> Vec<ReadLine> {
    let mut out = Vec::new();
    match level {
        ReadLevel::Normal => source_lines(content),
        ReadLevel::Compact => {
            out.push((
                None,
                "// RepoLens: language not recognized; using line-based compact fallback."
                    .to_string(),
            ));
            let mut last_written = 0;
            for (idx, line) in content.lines().enumerate() {
                let line_no = idx + 1;
                if line_no <= 40 || is_structural_line(line) {
                    if last_written + 1 < line_no {
                        out.push((None, "// ... lines omitted".to_string()));
                    }
                    out.push((Some(line_no), line.to_string()));
                    last_written = line_no;
                }
            }
            out
        }
        ReadLevel::Aggressive => {
            out.push((
                None,
                "// RepoLens: language not recognized; using line-based aggressive fallback."
                    .to_string(),
            ));
            for (idx, line) in content
                .lines()
                .enumerate()
                .filter(|(_, line)| is_structural_line(line))
            {
                out.push((Some(idx + 1), trim_body(line).to_string()));
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

fn format_lines(content: &[ReadLine], range: Option<&str>) -> Result<String> {
    let (start, end) = match range {
        Some(raw) => parse_line_range(raw)?,
        None => (1, usize::MAX),
    };

    let mut out = String::new();
    for (line_no, line) in content {
        let Some(line_no) = *line_no else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
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
        if line == 0 {
            bail!("line must be >= 1");
        }
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
    #[test]
    fn compact_preserves_multiline_function_signatures() {
        let temp = tempfile::tempdir().unwrap();
        let text = format!(
            "pub fn example(\n    value: i32,\n) -> i32 {{\n{}    value\n}}\n",
            "    let x = value;\n".repeat(25)
        );
        std::fs::write(temp.path().join("long.rs"), text).unwrap();
        let out = super::read_text_with_level(
            temp.path(),
            camino::Utf8Path::new("long.rs"),
            None,
            super::ReadLevel::Compact,
            None,
            None,
        )
        .unwrap();
        assert!(out.content.contains("2:     value: i32,"));
        assert!(out.content.contains("3: ) -> i32 {"));
        assert!(!out.content.contains("let x"));
        for (name, text, expected) in [
            (
                "long.js",
                format!(
                    "function example(\n    value\n) {{\n{}    return value;\n}}\n",
                    "    let x = value;\n".repeat(25)
                ),
                "3: ) {",
            ),
            (
                "long.py",
                format!(
                    "def example(\n    value: int,\n) -> int:\n{}    return value\n",
                    "    x = value\n".repeat(25)
                ),
                "3: ) -> int:",
            ),
        ] {
            std::fs::write(temp.path().join(name), text).unwrap();
            let out = super::read_text_with_level(
                temp.path(),
                camino::Utf8Path::new(name),
                None,
                super::ReadLevel::Compact,
                None,
                None,
            )
            .unwrap();
            assert!(out.content.contains(expected), "{}", out.content);
            assert!(out.content.contains("2:     value"));
            assert!(!out.content.contains("x = value"));
        }
    }

    #[test]
    fn explicit_ignored_file_is_parsed_without_persisting_it() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(temp.path().join("ignored.rs"), "fn ignored_function() {}\n").unwrap();
        let index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        crate::snapshot::save(&index).unwrap();
        let out = super::read_text_with_level(
            temp.path(),
            camino::Utf8Path::new("ignored.rs"),
            None,
            super::ReadLevel::Aggressive,
            None,
            None,
        )
        .unwrap();
        assert!(out.content.contains("ignored_function"));
        assert!(
            !crate::snapshot::load_or_build(temp.path())
                .unwrap()
                .files
                .iter()
                .any(|f| f.path == "ignored.rs")
        );
    }
    #[test]
    fn aggressive_limits_apply_after_source_range_for_parser_and_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let text = (1..=150)
            .map(|i| format!("fn f{i}() {{}}\n"))
            .collect::<String>();
        for name in ["many.rs", "many.unknown"] {
            std::fs::write(temp.path().join(name), &text).unwrap();
            let out = super::read_text_with_level(
                temp.path(),
                camino::Utf8Path::new(name),
                Some("130-135"),
                super::ReadLevel::Aggressive,
                None,
                None,
            )
            .unwrap();
            assert!(out.content.contains("130: fn f130()"), "{}", out.content);
            assert!(out.content.contains("135: fn f135()"));
            let full = super::read_text_with_level(
                temp.path(),
                camino::Utf8Path::new(name),
                None,
                super::ReadLevel::Aggressive,
                None,
                None,
            )
            .unwrap();
            assert!(!full.content.contains("121: fn f121()"));
        }
    }
    use camino::{Utf8Path, Utf8PathBuf};

    use super::{ReadLevel, parse_line_range, read_text, read_text_with_level};

    #[test]
    fn parses_line_ranges() {
        assert!(super::parse_line_range("0").is_err());
        assert_eq!(parse_line_range("2-4").unwrap(), (2, 4));
        assert_eq!(parse_line_range("3").unwrap(), (3, 3));
        assert!(parse_line_range("4-2").is_err());
    }

    #[test]
    fn sensitive_reads_require_explicit_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=fake\n").unwrap();
        assert!(read_text(dir.path(), Utf8Path::new(".env"), None, None, None).is_err());
        std::fs::write(
            dir.path().join(".repolensrc.toml"),
            "allow_sensitive = true\n",
        )
        .unwrap();
        assert!(
            read_text(dir.path(), Utf8Path::new(".env"), None, None, None)
                .unwrap()
                .contains("TOKEN=fake")
        );
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

    #[test]
    fn reduced_reads_preserve_source_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let body = (0..25)
            .map(|idx| format!("    let value_{idx} = {idx};"))
            .collect::<Vec<_>>()
            .join("\n");
        let source = format!("fn large() {{\n{body}\n}}\nfn sentinel() {{}}\n");
        for filename in ["main.rs", "main.unknown"] {
            std::fs::write(dir.path().join(filename), &source).unwrap();
            for level in [ReadLevel::Compact, ReadLevel::Aggressive] {
                let out = read_text_with_level(
                    dir.path(),
                    Utf8Path::new(filename),
                    None,
                    level,
                    None,
                    None,
                )
                .unwrap();
                assert!(
                    out.content.contains("28: fn sentinel()"),
                    "{filename} {level:?}: {}",
                    out.content
                );
                assert_eq!(out.fallback, filename.ends_with("unknown"));
                assert_eq!(
                    out.parser.as_deref(),
                    if out.fallback {
                        None
                    } else {
                        Some("tree-sitter-rust")
                    }
                );
                let ranged = read_text_with_level(
                    dir.path(),
                    Utf8Path::new(filename),
                    Some("28"),
                    level,
                    None,
                    None,
                )
                .unwrap();
                assert!(ranged.content.contains("28: fn sentinel()"));
                assert!(!ranged.content.contains("1: fn large"));
            }
        }
    }

    #[test]
    fn fallback_reads_preserve_coordinates_after_omissions() {
        let dir = tempfile::tempdir().unwrap();
        let source = format!("{}fn sentinel() {{}}\n", "ordinary text\n".repeat(59));
        std::fs::write(dir.path().join("main.unknown"), source).unwrap();
        for level in [ReadLevel::Compact, ReadLevel::Aggressive] {
            let out = read_text_with_level(
                dir.path(),
                Utf8Path::new("main.unknown"),
                Some("60-60"),
                level,
                None,
                None,
            )
            .unwrap();
            assert!(out.content.contains("60: fn sentinel()"));
            assert!(!out.content.contains("ordinary text"));
            assert!(out.fallback);
            assert!(out.parser.is_none());
        }
    }
}
