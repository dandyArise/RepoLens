use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

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

pub fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
                && !security::is_sensitive_name(&name)
        })
        .build();

    for item in walker {
        let entry = item?;
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && !security::is_sensitive_path(entry.path())
        {
            files.push(entry.into_path());
        }
    }

    Ok(files)
}

pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}
