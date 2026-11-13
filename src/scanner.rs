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
    source_files_targeted(root, config, None)
}

/// Traverse only the target's ancestor chain, using exactly the full scanner's
/// ignore, hidden, sensitive, symlink and size policies.
pub fn is_source_file(root: &Path, path: &Path, config: &Config) -> Result<bool> {
    Ok(
        source_files_targeted(root, config, Some(path.to_path_buf()))?
            .iter()
            .any(|candidate| candidate == path),
    )
}

fn source_files_targeted(
    root: &Path,
    config: &Config,
    target: Option<PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let allow_sensitive = config.allow_sensitive;
    let filter_hidden = !config.include_hidden;
    let walker = WalkBuilder::new(root)
        .hidden(filter_hidden)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(move |entry| {
            if target
                .as_ref()
                .is_some_and(|path| !path.starts_with(entry.path()))
            {
                return false;
            }
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
    fn targeted_scan_matches_full_scan_policies_and_inherited_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::create_dir_all(root.join("secrets")).unwrap();
        fs::write(temp.path().join(".ignore"), "*.py\n").unwrap();
        fs::write(root.join(".ignore"), "!/*.py\n").unwrap();
        fs::write(root.join("nested/.ignore"), "!keep.py\n").unwrap();
        fs::write(root.join(".gitignore"), "blocked.rs\n").unwrap();
        let paths = [
            "main.py",
            "nested/no.py",
            "nested/keep.py",
            "blocked.rs",
            "target/a.rs",
            ".hidden/a.rs",
            ".hidden.rs",
            "secrets/a.rs",
            ".env",
            "large.rs",
        ];
        for path in paths {
            fs::write(root.join(path), "token\n").unwrap();
        }
        fs::write(root.join("large.rs"), "x".repeat(128)).unwrap();
        for include_hidden in [false, true] {
            for allow_sensitive in [false, true] {
                let config = Config {
                    include_hidden,
                    allow_sensitive,
                    max_file_size: 64,
                };
                let full = source_files(&root, &config).unwrap();
                assert!(full.contains(&root.join("main.py")));
                assert!(full.contains(&root.join("nested/keep.py")));
                for path in paths.into_iter().chain(["absent.rs"]) {
                    let path = root.join(path);
                    assert_eq!(
                        super::is_source_file(&root, &path, &config).unwrap(),
                        full.contains(&path),
                        "{path:?}"
                    );
                }
            }
        }
    }

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
        fs::write(temp.path().join(".hidden.rs"), "fn hidden() {}\n").unwrap();
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
        assert!(!names.contains(&".hidden.rs"));
        assert!(!names.contains(&"ignored.txt"));
        assert!(!names.contains(&".env"));
        assert!(!names.contains(&"api.json"));
    }

    #[test]
    fn can_include_hidden_files_when_configured() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".hidden.rs"), "fn hidden() {}\n").unwrap();

        let files = source_files(
            temp.path(),
            &Config {
                include_hidden: true,
                ..Config::default()
            },
        )
        .unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|path| path.file_name()?.to_str())
            .collect();

        assert!(names.contains(&".hidden.rs"));
    }
}
