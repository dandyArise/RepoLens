use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

use crate::config::Config;
use crate::security;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".repolens",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    ".venv",
    "__pycache__",
];

pub fn source_files(root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let allow_sensitive = config.allow_sensitive;
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
                && (allow_sensitive || !security::is_sensitive_name(&name))
        })
        .build();

    for item in walker {
        let entry = item?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if !config.allow_sensitive && security::is_sensitive_path(entry.path()) {
            continue;
        }
        if entry
            .metadata()
            .is_ok_and(|metadata| metadata.len() > config.max_file_size)
        {
            continue;
        }
        {
            files.push(entry.into_path());
        }
    }

    Ok(files)
}

pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::config::Config;

    use super::{looks_binary, source_files};

    #[test]
    fn detects_binary_content() {
        assert!(looks_binary(b"abc\0def"));
        assert!(!looks_binary(b"abcdef"));
    }

    #[test]
    fn respects_gitignore_and_sensitive_defaults() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(temp.path().join("ignored.txt"), "skip\n").unwrap();
        fs::write(temp.path().join(".env"), "SECRET=1\n").unwrap();
        fs::create_dir(temp.path().join("secrets")).unwrap();
        fs::write(temp.path().join("secrets").join("api.json"), "{}\n").unwrap();

        let files = source_files(temp.path(), &Config::default()).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|path| path.file_name()?.to_str())
            .collect();

        assert!(names.contains(&"main.rs"));
        assert!(!names.contains(&"ignored.txt"));
        assert!(!names.contains(&".env"));
        assert!(!names.contains(&"api.json"));
    }
}
