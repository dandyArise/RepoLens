use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::config::Config;
use crate::pathing::canonical_utf8;
use crate::scanner;

const MAX_IMPORTANT_LINES_PER_FILE: usize = 8;

#[derive(Debug, Serialize)]
pub struct TestsAwareReport {
    root: String,
    totals: TestTotals,
    frameworks: Vec<String>,
    test_files: Vec<TestFileReport>,
    confidence: &'static str,
    notes: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct TestTotals {
    files: usize,
    assertions: usize,
    fixtures: usize,
    mocks: usize,
}

#[derive(Debug, Serialize)]
struct TestFileReport {
    path: String,
    language: String,
    detected_by: Vec<String>,
    frameworks: Vec<String>,
    assertions: usize,
    fixtures: usize,
    mocks: usize,
    important_lines: Vec<ImportantLine>,
}

#[derive(Debug, Serialize)]
struct ImportantLine {
    line: usize,
    kind: &'static str,
    text: String,
}

#[derive(Default)]
struct FileSignals {
    detected_by: BTreeSet<String>,
    frameworks: BTreeSet<String>,
    assertions: usize,
    fixtures: usize,
    mocks: usize,
    important_lines: Vec<ImportantLine>,
}

pub fn print(root: &Path) -> Result<()> {
    let report = analyze(root)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub fn analyze(root: &Path) -> Result<TestsAwareReport> {
    let root = canonical_utf8(root)?;
    let config = Config::load(root.as_std_path())?;
    let files = scanner::source_files(root.as_std_path(), &config)?;
    build_report(&root, files)
}

fn build_report(root: &Utf8Path, files: Vec<std::path::PathBuf>) -> Result<TestsAwareReport> {
    let mut reports = Vec::new();
    let mut frameworks = BTreeSet::new();
    let mut totals = TestTotals::default();

    for path in files {
        let rel = Utf8PathBuf::from_path_buf(
            path.strip_prefix(root.as_std_path())
                .unwrap_or(path.as_path())
                .to_path_buf(),
        )
        .map_err(|path| anyhow::anyhow!("non-utf8 path {}", path.display()))?;
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let signals = analyze_file(&rel, &text);
        if signals.detected_by.is_empty() {
            continue;
        }

        for framework in &signals.frameworks {
            frameworks.insert(framework.clone());
        }
        totals.files += 1;
        totals.assertions += signals.assertions;
        totals.fixtures += signals.fixtures;
        totals.mocks += signals.mocks;

        reports.push(TestFileReport {
            path: display_path(&rel),
            language: language_name(&rel).to_string(),
            detected_by: signals.detected_by.into_iter().collect(),
            frameworks: signals.frameworks.into_iter().collect(),
            assertions: signals.assertions,
            fixtures: signals.fixtures,
            mocks: signals.mocks,
            important_lines: signals.important_lines,
        });
    }

    reports.sort_by(|a, b| a.path.cmp(&b.path));
    let confidence = if totals.files > 0 { "high" } else { "medium" };
    let notes = if totals.files > 0 {
        vec!["Detected test files using path and content heuristics.".to_string()]
    } else {
        vec!["No test files detected by generic path or content heuristics.".to_string()]
    };

    Ok(TestsAwareReport {
        root: root.to_string(),
        totals,
        frameworks: frameworks.into_iter().collect(),
        test_files: reports,
        confidence,
        notes,
    })
}

fn analyze_file(path: &Utf8Path, text: &str) -> FileSignals {
    let mut signals = FileSignals::default();
    let mut in_test_context = is_test_path(path);

    if in_test_context {
        signals.detected_by.insert("path".to_string());
    }

    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();

        for framework in detect_frameworks(path, trimmed, &lower) {
            signals.detected_by.insert("content".to_string());
            signals.frameworks.insert(framework.to_string());
            push_line(&mut signals, line_no, "framework", trimmed);
            in_test_context = true;
        }

        if !in_test_context {
            continue;
        }

        if is_fixture_line(trimmed, &lower) {
            signals.detected_by.insert("content".to_string());
            signals.fixtures += 1;
            push_line(&mut signals, line_no, "fixture", trimmed);
        }

        if is_mock_line(trimmed, &lower) {
            signals.detected_by.insert("content".to_string());
            signals.mocks += 1;
            push_line(&mut signals, line_no, "mock", trimmed);
        }

        if is_assertion_line(trimmed, &lower) {
            signals.detected_by.insert("content".to_string());
            signals.assertions += 1;
            push_line(&mut signals, line_no, "assertion", trimmed);
        }
    }

    signals
}

fn push_line(signals: &mut FileSignals, line: usize, kind: &'static str, text: &str) {
    if signals.important_lines.len() >= MAX_IMPORTANT_LINES_PER_FILE {
        return;
    }
    let text = text.chars().take(160).collect();
    signals
        .important_lines
        .push(ImportantLine { line, kind, text });
}

fn is_test_path(path: &Utf8Path) -> bool {
    let normalized = path.as_str().replace('\\', "/").to_ascii_lowercase();
    let file = path
        .file_name()
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_default();

    normalized.contains("/tests/")
        || normalized.contains("/test/")
        || normalized.contains("/__tests__/")
        || file.starts_with("test_")
        || file.ends_with("_test.rs")
        || file.ends_with("_test.go")
        || file.ends_with("_test.py")
        || file.ends_with(".test.js")
        || file.ends_with(".test.ts")
        || file.ends_with(".test.tsx")
        || file.ends_with(".spec.js")
        || file.ends_with(".spec.ts")
        || file.ends_with(".spec.tsx")
        || file.ends_with("test.java")
        || file.ends_with("tests.cs")
}

fn detect_frameworks<'a>(path: &Utf8Path, line: &'a str, lower: &str) -> Vec<&'a str> {
    let mut frameworks = Vec::new();
    let ext = path.extension().unwrap_or_default();

    if ext == "rs"
        && (line == "#[test]"
            || line.starts_with("#[tokio::test")
            || line.starts_with("#[async_std::test"))
    {
        frameworks.push("rust-test");
    }
    if matches!(ext, "py" | "pyw") && (lower.contains("pytest") || line.contains("@pytest.fixture"))
    {
        frameworks.push("pytest");
    }
    if matches!(ext, "py" | "pyw")
        && (lower.contains("unittest") || line.contains("unittest.TestCase"))
    {
        frameworks.push("unittest");
    }
    if is_js_like(ext)
        && (has_call(line, "describe") || has_call(line, "it") || has_call(line, "test"))
    {
        frameworks.push("js-test-runner");
    }
    if is_js_like(ext) && lower.contains("jest") {
        frameworks.push("jest");
    }
    if is_js_like(ext) && (lower.contains("vitest") || line.contains("vi.")) {
        frameworks.push("vitest");
    }
    if ext == "go" && (line.contains("func Test") || line.contains("testing.T")) {
        frameworks.push("go-test");
    }
    if ext == "java" && (line.contains("@Test") || lower.contains("org.junit")) {
        frameworks.push("junit");
    }
    if ext == "cs" && (line.contains("[Fact]") || line.contains("[Theory]")) {
        frameworks.push("xunit");
    }
    if ext == "cs" && (line.contains("[Test]") || lower.contains("nunit")) {
        frameworks.push("nunit");
    }
    if ext == "php" && lower.contains("phpunit") {
        frameworks.push("phpunit");
    }
    if ext == "rb" && (line.contains("RSpec.describe") || line.starts_with("describe ")) {
        frameworks.push("rspec");
    }
    frameworks
}

fn is_js_like(ext: &str) -> bool {
    matches!(
        ext,
        "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "tsx" | "jsx"
    )
}

fn has_call(line: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut offset = 0;
    while let Some(pos) = line[offset..].find(&needle) {
        let absolute = offset + pos;
        let previous = line[..absolute].chars().next_back();
        if previous.is_none_or(|ch| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_')) {
            return true;
        }
        offset = absolute + needle.len();
    }
    false
}

fn is_fixture_line(line: &str, lower: &str) -> bool {
    line.contains("@pytest.fixture")
        || lower.contains("fixture")
        || lower.contains("beforeeach")
        || lower.contains("before_each")
        || lower.contains("beforeall")
        || lower.contains("aftereach")
        || lower.contains("after_each")
        || lower.contains("afterall")
        || line.starts_with("setUp(")
        || line.starts_with("def setUp")
        || line.starts_with("tearDown(")
        || line.starts_with("def tearDown")
}

fn is_mock_line(line: &str, lower: &str) -> bool {
    lower.contains("mock")
        || lower.contains("monkeypatch")
        || lower.contains("stub")
        || lower.contains("spy")
        || lower.contains("fake")
        || line.contains("jest.fn")
        || line.contains("vi.fn")
        || line.contains("Mockito.")
}

fn is_assertion_line(line: &str, lower: &str) -> bool {
    line.starts_with("assert ")
        || line.contains(" assert ")
        || line.contains("assert!(")
        || line.contains("assert_eq!(")
        || line.contains("assert_ne!(")
        || line.contains("expect(")
        || line.contains("assertThat(")
        || lower.contains("assert_")
        || lower.contains("require.")
}

fn language_name(path: &Utf8Path) -> &'static str {
    match path.extension() {
        Some("rs") => "Rust",
        Some("py") | Some("pyw") => "Python",
        Some("js") | Some("mjs") | Some("cjs") => "JavaScript",
        Some("ts") | Some("mts") | Some("cts") => "TypeScript",
        Some("tsx") | Some("jsx") => "React",
        Some("go") => "Go",
        Some("java") => "Java",
        Some("cs") => "C#",
        Some("php") => "PHP",
        Some("rb") => "Ruby",
        _ => "Unknown",
    }
}

fn display_path(path: &Utf8Path) -> String {
    path.as_str().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{analyze, analyze_file, is_test_path};

    #[test]
    fn detects_test_paths_generically() {
        assert!(is_test_path(camino::Utf8Path::new("tests/api_test.py")));
        assert!(is_test_path(camino::Utf8Path::new("src/foo.test.ts")));
        assert!(is_test_path(camino::Utf8Path::new("pkg/service_test.go")));
        assert!(!is_test_path(camino::Utf8Path::new("src/main.rs")));
    }

    #[test]
    fn detects_frameworks_fixtures_mocks_and_assertions() {
        let text = r#"
import pytest
@pytest.fixture
def client():
    return Mock()

def test_fetch(monkeypatch):
    assert client is not None
"#;

        let report = analyze_file(camino::Utf8Path::new("tests/test_fetcher.py"), text);

        assert!(report.detected_by.contains("path"));
        assert!(report.detected_by.contains("content"));
        assert!(report.frameworks.contains("pytest"));
        assert!(report.fixtures >= 1);
        assert!(report.mocks >= 2);
        assert!(report.assertions >= 1);
        assert!(!report.important_lines.is_empty());
    }

    #[test]
    fn builds_repo_level_report() {
        let temp = tempfile::tempdir().unwrap();
        let tests = temp.path().join("tests");
        fs::create_dir(&tests).unwrap();
        fs::write(
            tests.join("api.test.ts"),
            "import { describe, expect, it, vi } from 'vitest';\nit('works', () => { const fn = vi.fn(); expect(fn).toBeDefined(); });\n",
        )
        .unwrap();
        fs::write(temp.path().join("favicon.png"), b"\x89PNG\r\n\x1a\n\0").unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

        let report = analyze(temp.path()).unwrap();

        assert_eq!(report.totals.files, 1);
        assert!(report.frameworks.contains(&"vitest".to_string()));
        assert_eq!(report.test_files[0].path, "tests/api.test.ts");
        assert_eq!(report.confidence, "high");
    }
}
