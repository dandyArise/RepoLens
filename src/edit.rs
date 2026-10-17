use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::cli::EditOpArg;
use crate::config::Config;
use crate::pathing::{canonical_utf8, resolve_in_root};
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
    // A stable OS-backed lock serializes guarded edits, including index updates.
    // Locking the target itself would stop protecting it after atomic replacement.
    let lock_dir = root.join(".repolens");
    fs::create_dir_all(&lock_dir).context("failed to create edit lock directory")?;
    let edit_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_dir.join("edit.lock"))
        .context("failed to open edit lock")?;
    edit_lock.lock().context("failed to acquire edit lock")?;
    let config = Config::load(root.as_std_path())?;
    let (target, rel_path) = resolve_in_root(&root, path)?;
    if rel_path
        .components()
        .next()
        .is_some_and(|component| component.as_str().eq_ignore_ascii_case(".repolens"))
    {
        bail!("refusing to edit RepoLens internal files");
    }

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
    index.refresh_file(&rel_path)?;
    snapshot::save(&index)?;

    Ok(EditResult {
        path: rel_path,
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
            if !insertion.is_empty()
                && start == lines.len() + 1
                && let Some(last) = lines.last_mut()
                && !last.ends_with('\n')
            {
                let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
                last.push_str(newline);
            }
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
    let mut tmp = tempfile::Builder::new()
        .prefix(".repolens-edit-")
        .tempfile_in(parent)
        .with_context(|| format!("failed to create temp file in {parent}"))?;
    tmp.write_all(bytes)
        .with_context(|| format!("failed to write temp file for {path}"))?;
    tmp.as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {path}"))?;
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
    fn appends_a_line_after_unterminated_last_line() {
        assert_eq!(
            apply_to_text("a", EditOpArg::Insert, 2, None, Some("b\n")).unwrap(),
            "a\nb\n"
        );
        assert_eq!(
            apply_to_text("a", EditOpArg::Insert, 2, None, Some("b")).unwrap(),
            "a\nb"
        );
        assert_eq!(
            apply_to_text("a\r\nb", EditOpArg::Insert, 3, None, Some("c\r\n")).unwrap(),
            "a\r\nb\r\nc\r\n"
        );
        assert_eq!(
            apply_to_text("a", EditOpArg::Insert, 2, None, Some("")).unwrap(),
            "a"
        );
        assert_eq!(
            apply_to_text("", EditOpArg::Insert, 1, None, Some("b\n")).unwrap(),
            "b\n"
        );
    }

    #[test]
    fn edit_preserves_preexisting_temporary_sibling() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.txt");
        let sibling = temp.path().join(".sample.txt.repolens-tmp");
        fs::write(&path, "a\n").unwrap();
        fs::write(&sibling, "user data\n").unwrap();
        let hash = blake3::hash(b"a\n").to_hex().to_string();
        super::apply(
            temp.path(),
            Utf8Path::new("sample.txt"),
            EditOpArg::Replace,
            1,
            None,
            Some("b\n"),
            &hash,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(sibling).unwrap(), "user data\n");
        assert_eq!(fs::read_to_string(path).unwrap(), "b\n");
    }

    #[test]
    fn cannot_replace_the_repository_lock_through_an_edit() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".repolens")).unwrap();
        let lock = temp.path().join(".repolens/edit.lock");
        fs::write(&lock, "original lock\n").unwrap();
        let hash = blake3::hash(b"original lock\n").to_hex().to_string();
        let error = super::apply(
            temp.path(),
            Utf8Path::new(".repolens/edit.lock"),
            EditOpArg::Replace,
            1,
            None,
            Some("replaced\n"),
            &hash,
        )
        .unwrap_err();
        assert!(error.to_string().contains("internal files"));
        assert_eq!(fs::read_to_string(lock).unwrap(), "original lock\n");
    }

    #[test]
    fn concurrent_guarded_edits_accept_only_one_writer() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.txt");
        let text = format!("original\n{}", "padding\n".repeat(100_000));
        fs::write(&path, &text).unwrap();
        let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        let barrier = std::sync::Barrier::new(4);
        let results = std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|writer| {
                    let root = temp.path();
                    let hash = &hash;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let content = format!("writer_{writer}\n");
                        (
                            writer,
                            super::apply(
                                root,
                                Utf8Path::new("sample.txt"),
                                EditOpArg::Replace,
                                1,
                                None,
                                Some(&content),
                                hash,
                            ),
                        )
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let winners = results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1, "{results:?}");
        for (_, result) in &results {
            if let Err(error) = result {
                assert!(error.to_string().contains("hash mismatch"), "{error}");
            }
        }
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .starts_with(&format!("writer_{}\n", winners[0].0))
        );
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
