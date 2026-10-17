use std::fs;
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use memmap2::MmapOptions;
use serde::Serialize;

use crate::index::{INDEX_VERSION, ProjectIndex};
use crate::pathing::canonical_utf8;
use crate::{config::Config, security};

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
    let config = Config::load(root.as_std_path())?;
    let binary_path = binary_index_path(&root);
    let index_path = index_path(&root);
    if binary_is_current(&binary_path, &index_path)
        && let Ok(bytes) = mmap_file(&binary_path)
        && let Ok((index, _)) = bincode::serde::decode_from_slice::<ProjectIndex, _>(
            bytes.as_ref(),
            bincode::config::standard(),
        )
        && index.root == root
        && index.version == INDEX_VERSION
        && matches_sensitive_policy(&index, &config)
    {
        return Ok(index);
    }

    match fs::read_to_string(&index_path) {
        Ok(raw) => match serde_json::from_str::<ProjectIndex>(&raw) {
            Ok(index)
                if index.root == root
                    && index.version == INDEX_VERSION
                    && matches_sensitive_policy(&index, &config) =>
            {
                Ok(index)
            }
            _ => ProjectIndex::build(root.as_std_path()),
        },
        Err(_) => ProjectIndex::build(root.as_std_path()),
    }
}

fn matches_sensitive_policy(index: &ProjectIndex, config: &Config) -> bool {
    index.indexed_config == *config
        && (config.allow_sensitive
            || !index
                .files
                .iter()
                .any(|file| security::is_sensitive_path(index.root.join(&file.path).as_std_path())))
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

pub fn stamp(root: &Utf8Path) -> Vec<Option<(std::time::SystemTime, u64)>> {
    [
        index_path(root),
        binary_index_path(root),
        root.join(".repolensrc.toml"),
    ]
    .iter()
    .map(|path| {
        let metadata = fs::metadata(path).ok()?;
        Some((metadata.modified().ok()?, metadata.len()))
    })
    .collect()
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
    #[test]
    fn config_filter_changes_rebuild_binary_and_json_snapshots() {
        for binary in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let config = temp.path().join(".repolensrc.toml");
            fs::write(&config, "include_hidden=true\n").unwrap();
            fs::write(temp.path().join(".hidden.rs"), "fn hidden() {}\n").unwrap();
            let index = ProjectIndex::build(temp.path()).unwrap();
            save(&index).unwrap();
            if !binary {
                fs::remove_file(binary_index_path(&index.root)).unwrap();
            }
            fs::write(&config, "include_hidden=false\n").unwrap();
            assert!(
                !load_or_build(temp.path())
                    .unwrap()
                    .files
                    .iter()
                    .any(|f| f.path == ".hidden.rs")
            );
            fs::write(&config, "include_hidden=true\nmax_file_size='1b'\n").unwrap();
            assert!(load_or_build(temp.path()).unwrap().files.is_empty());
        }
    }
    use std::fs;

    use crate::index::ProjectIndex;

    use super::{binary_index_path, info, load_or_build, save};

    #[test]
    fn rebuilds_legacy_dependency_indexes_in_binary_and_json() {
        for binary in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("main.js"), "require('./util');\n").unwrap();
            fs::write(temp.path().join("util.js"), "module.exports=1;\n").unwrap();
            fs::write(temp.path().join("util.ts"), "export const n=2;\n").unwrap();
            let mut legacy = ProjectIndex::build(temp.path()).unwrap();
            legacy.version = 6;
            let wrong_id = legacy
                .files
                .iter()
                .find(|f| f.path == "util.ts")
                .unwrap()
                .id;
            let import = &mut legacy
                .deps
                .iter_mut()
                .find(|d| d.path == "main.js")
                .unwrap()
                .imports[0];
            import.resolved_path = Some("util.ts".into());
            import.resolved_file_id = Some(wrong_id);
            save(&legacy).unwrap();
            if !binary {
                fs::remove_file(binary_index_path(&legacy.root)).unwrap();
            }
            let loaded = load_or_build(temp.path()).unwrap();
            let import = &loaded
                .deps
                .iter()
                .find(|d| d.path == "main.js")
                .unwrap()
                .imports[0];
            assert_eq!(
                import.resolved_path.as_deref(),
                Some(camino::Utf8Path::new("util.js"))
            );
        }
    }

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

    #[test]
    fn revoked_sensitive_policy_applies_to_binary_and_json_snapshots() {
        for binary in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let config = temp.path().join(".repolensrc.toml");
            fs::write(&config, "allow_sensitive = true\ninclude_hidden = true\n").unwrap();
            fs::write(temp.path().join(".env"), "SYNTHETIC_MARKER=example\n").unwrap();
            let index = ProjectIndex::build(temp.path()).unwrap();
            assert!(index.files.iter().any(|file| file.path == ".env"));
            save(&index).unwrap();
            if !binary {
                fs::remove_file(binary_index_path(&index.root)).unwrap();
            }
            fs::write(&config, "allow_sensitive = false\ninclude_hidden = true\n").unwrap();
            let loaded = load_or_build(temp.path()).unwrap();
            assert!(!loaded.files.iter().any(|file| file.path == ".env"));
            assert!(crate::search::word_paths(&loaded, "SYNTHETIC_MARKER", 10).is_empty());
        }
    }

    #[test]
    fn copied_snapshot_cannot_redirect_reads_to_another_root() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(source.path().join("source.rs"), "fn source() {}\n").unwrap();
        let index = ProjectIndex::build(source.path()).unwrap();
        save(&index).unwrap();
        fs::write(target.path().join("target.rs"), "fn target() {}\n").unwrap();
        fs::create_dir(target.path().join(".repolens")).unwrap();
        for name in ["index.json", "index.bin"] {
            fs::copy(
                source.path().join(".repolens").join(name),
                target.path().join(".repolens").join(name),
            )
            .unwrap();
        }
        let loaded = load_or_build(target.path()).unwrap();
        assert_eq!(
            loaded.root.as_std_path(),
            dunce::canonicalize(target.path()).unwrap()
        );
        assert_eq!(loaded.files[0].path.as_str(), "target.rs");
    }
}
