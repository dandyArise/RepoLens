use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::index::ProjectIndex;
use crate::pathing::canonical_utf8;

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
