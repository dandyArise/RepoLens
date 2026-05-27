use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::index::ProjectIndex;
use crate::pathing::canonical_utf8;

#[derive(Debug, Serialize)]
pub struct SnapshotInfo {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub bytes: u64,
    pub version: u32,
    pub files: usize,
    pub symbols: usize,
    pub deps_files: usize,
}

pub fn load_or_build(root: &Path) -> Result<ProjectIndex> {
    let root = canonical_utf8(root)?;
    let index_path = index_path(&root);
    match fs::read_to_string(&index_path) {
        Ok(raw) => serde_json::from_str(&raw).or_else(|_| ProjectIndex::build(root.as_std_path())),
        Err(_) => ProjectIndex::build(root.as_std_path()),
    }
}

pub fn save(index: &ProjectIndex) -> Result<()> {
    let dir = index.root.join(".repolens");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("index.json");
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&path, json).with_context(|| format!("failed to write {path}"))?;
    Ok(())
}

pub fn index_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".repolens").join("index.json")
}

pub fn info(index: &ProjectIndex) -> SnapshotInfo {
    let path = index_path(&index.root);
    let metadata = fs::metadata(path.as_std_path()).ok();
    SnapshotInfo {
        path,
        exists: metadata.is_some(),
        bytes: metadata.map(|metadata| metadata.len()).unwrap_or(0),
        version: index.version,
        files: index.files.len(),
        symbols: index.symbols.len(),
        deps_files: index.deps.len(),
    }
}

pub fn print_info(index: &ProjectIndex) {
    let info = info(index);
    println!("path: {}", info.path);
    println!("exists: {}", info.exists);
    println!("bytes: {}", info.bytes);
    println!("version: {}", info.version);
    println!("files: {}", info.files);
    println!("symbols: {}", info.symbols);
    println!("deps files: {}", info.deps_files);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::index::ProjectIndex;

    use super::{load_or_build, save};

    #[test]
    fn saves_and_loads_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

        let index = ProjectIndex::build(temp.path()).unwrap();
        save(&index).unwrap();

        let loaded = load_or_build(temp.path()).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files[0].path.as_str(), "main.rs");
    }
}
