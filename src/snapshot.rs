use std::fs;
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use memmap2::MmapOptions;
use serde::Serialize;

use crate::index::ProjectIndex;
use crate::pathing::canonical_utf8;

#[derive(Debug, Serialize)]
pub struct SnapshotInfo {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub bytes: u64,
    pub binary_path: Utf8PathBuf,
    pub binary_exists: bool,
    pub binary_bytes: u64,
    pub version: u32,
    pub files: usize,
    pub symbols: usize,
    pub deps_files: usize,
}

pub fn load_or_build(root: &Path) -> Result<ProjectIndex> {
    let root = canonical_utf8(root)?;
    let binary_path = binary_index_path(&root);
    let index_path = index_path(&root);
    if binary_is_current(&binary_path, &index_path)
        && let Ok(bytes) = mmap_file(&binary_path)
        && let Ok((index, _)) =
            bincode::serde::decode_from_slice(bytes.as_ref(), bincode::config::standard())
    {
        return Ok(index);
    }

    match fs::read_to_string(&index_path) {
        Ok(raw) => serde_json::from_str(&raw).or_else(|_| ProjectIndex::build(root.as_std_path())),
        Err(_) => ProjectIndex::build(root.as_std_path()),
    }
}

fn mmap_file(path: &Utf8Path) -> Result<memmap2::Mmap> {
    let file = File::open(path).with_context(|| format!("failed to open {path}"))?;
    // The map is read-only and the file handle is not exposed for mutation.
    unsafe { MmapOptions::new().map(&file) }.with_context(|| format!("failed to mmap {path}"))
}

pub fn save(index: &ProjectIndex) -> Result<()> {
    let dir = index.root.join(".repolens");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("index.json");
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&path, json).with_context(|| format!("failed to write {path}"))?;

    let binary_path = dir.join("index.bin");
    let binary = bincode::serde::encode_to_vec(index, bincode::config::standard())?;
    let _ = fs::write(&binary_path, binary);
    Ok(())
}

fn binary_is_current(binary_path: &Utf8Path, index_path: &Utf8Path) -> bool {
    let Ok(binary_modified) = fs::metadata(binary_path.as_std_path()).and_then(|m| m.modified())
    else {
        return false;
    };
    let Ok(index_modified) = fs::metadata(index_path.as_std_path()).and_then(|m| m.modified())
    else {
        return true;
    };
    binary_modified >= index_modified
}

pub fn index_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".repolens").join("index.json")
}

pub fn binary_index_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".repolens").join("index.bin")
}

pub fn info(index: &ProjectIndex) -> SnapshotInfo {
    let path = index_path(&index.root);
    let metadata = fs::metadata(path.as_std_path()).ok();
    let binary_path = binary_index_path(&index.root);
    let binary_metadata = fs::metadata(binary_path.as_std_path()).ok();
    SnapshotInfo {
        path,
        exists: metadata.is_some(),
        bytes: metadata.map(|metadata| metadata.len()).unwrap_or(0),
        binary_path,
        binary_exists: binary_metadata.is_some(),
        binary_bytes: binary_metadata.map(|metadata| metadata.len()).unwrap_or(0),
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
    println!("binary path: {}", info.binary_path);
    println!("binary exists: {}", info.binary_exists);
    println!("binary bytes: {}", info.binary_bytes);
    println!("version: {}", info.version);
    println!("files: {}", info.files);
    println!("symbols: {}", info.symbols);
    println!("deps files: {}", info.deps_files);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::index::ProjectIndex;

    use super::{binary_index_path, info, load_or_build, save};

    #[test]
    fn saves_and_loads_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

        let index = ProjectIndex::build(temp.path()).unwrap();
        save(&index).unwrap();

        let loaded = load_or_build(temp.path()).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files[0].path.as_str(), "main.rs");
        assert!(binary_index_path(&loaded.root).exists());
        assert!(info(&loaded).binary_bytes > 0);
    }
}
