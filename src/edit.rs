use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::cli::EditOpArg;
use crate::config::Config;
use crate::pathing::{canonical_utf8, safe_join};
use crate::security;
use crate::snapshot;

#[derive(Debug, Clone, Serialize)]
pub struct EditResult {
    pub path: Utf8PathBuf,
    pub hash: String,
    pub lines: usize,
}

pub fn apply(
    root: &Path,
    path: &Utf8Path,
    op: EditOpArg,
    start: usize,
    end: Option<usize>,
    content: Option<&str>,
    expected_hash: &str,
) -> Result<EditResult> {
    if expected_hash.trim().is_empty() {
        bail!("hash is required for edits");
    }

    let root = canonical_utf8(root)?;
    let config = Config::load(root.as_std_path())?;
    let target = safe_join(&root, path)?;

    if !config.allow_sensitive && security::is_sensitive_path(target.as_std_path()) {
        bail!("refusing to edit sensitive path");
    }

    let bytes = fs::read(&target).with_context(|| format!("failed to read {target}"))?;
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if actual_hash != expected_hash {
        bail!("hash mismatch");
    }

    let text = String::from_utf8(bytes).context("refusing to edit non-utf8 file")?;
    let next = apply_to_text(&text, op, start, end, content)?;
    let next_hash = blake3::hash(next.as_bytes()).to_hex().to_string();

    atomic_write(&target, next.as_bytes())?;
    let mut index = snapshot::load_or_build(root.as_std_path())?;
    index.refresh_file(&path.to_path_buf())?;
    snapshot::save(&index)?;

    Ok(EditResult {
        path: path.to_path_buf(),
        hash: next_hash,
        lines: next.lines().count(),
    })
}

fn apply_to_text(
    text: &str,
    op: EditOpArg,
    start: usize,
    end: Option<usize>,
    content: Option<&str>,
) -> Result<String> {
    if start == 0 {
        bail!("start must be >= 1");
    }

    let mut lines: Vec<String> = split_preserve_lines(text);
    match op {
        EditOpArg::Replace => {
            let end = end.unwrap_or(start);
            validate_range(lines.len(), start, end)?;
            let replacement = content.ok_or_else(|| anyhow::anyhow!("content is required"))?;
            lines.splice(start - 1..end, split_preserve_lines(replacement));
        }
        EditOpArg::Insert => {
            if start > lines.len() + 1 {
                bail!("insert start out of range");
            }
            let insertion = content.ok_or_else(|| anyhow::anyhow!("content is required"))?;
            lines.splice(start - 1..start - 1, split_preserve_lines(insertion));
        }
        EditOpArg::Delete => {
            let end = end.unwrap_or(start);
            validate_range(lines.len(), start, end)?;
            lines.drain(start - 1..end);
        }
    }

    Ok(lines.concat())
}

fn split_preserve_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').map(str::to_string).collect()
}

fn validate_range(line_count: usize, start: usize, end: usize) -> Result<()> {
    if start == 0 || end < start || end > line_count {
        bail!("line range out of bounds");
    }
    Ok(())
}

fn atomic_write(path: &Utf8Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target has no parent"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("target has no file name"))?;
    let tmp = parent.join(format!(".{file_name}.repolens-tmp"));
    fs::write(&tmp, bytes).with_context(|| format!("failed to write temp file {tmp}"))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {path}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8Path;

    use super::apply_to_text;
    use crate::cli::EditOpArg;

    #[test]
    fn replaces_lines() {
        let out = apply_to_text("a\nb\nc\n", EditOpArg::Replace, 2, Some(2), Some("B\n")).unwrap();
        assert_eq!(out, "a\nB\nc\n");
    }

    #[test]
    fn inserts_lines() {
        let out = apply_to_text("a\nc\n", EditOpArg::Insert, 2, None, Some("b\n")).unwrap();
        assert_eq!(out, "a\nb\nc\n");
    }

    #[test]
    fn deletes_lines() {
        let out = apply_to_text("a\nb\nc\n", EditOpArg::Delete, 2, Some(2), None).unwrap();
        assert_eq!(out, "a\nc\n");
    }

    #[test]
    fn applies_file_edit_with_hash_guard() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.rs");
        fs::write(&path, "fn main() {}\n").unwrap();
        let hash = blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string();

        let result = super::apply(
            temp.path(),
            Utf8Path::new("sample.rs"),
            EditOpArg::Replace,
            1,
            Some(1),
            Some("fn edited() {}\n"),
            &hash,
        )
        .unwrap();

        assert_eq!(result.lines, 1);
        assert_eq!(fs::read_to_string(path).unwrap(), "fn edited() {}\n");
        assert!(temp.path().join(".repolens/index.json").exists());
    }
}
