use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::pathing::canonical_utf8;
use crate::scanner;
use crate::search;

pub type FileId = u32;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub root: Utf8PathBuf,
    pub files: Vec<FileEntry>,
    pub words: BTreeMap<String, Vec<FileId>>,
    pub trigrams: BTreeMap<String, Vec<FileId>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: FileId,
    pub path: Utf8PathBuf,
    pub bytes: u64,
    pub lines: u32,
    pub mtime_ms: u64,
    pub hash: String,
}

impl ProjectIndex {
    pub fn build(root: &Path) -> Result<Self> {
        let root = canonical_utf8(root)?;
        let mut files = Vec::new();
        let mut words: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();
        let mut trigrams: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();

        for path in scanner::source_files(root.as_std_path())? {
            let bytes =
                fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            if scanner::looks_binary(&bytes) {
                continue;
            }

            let rel = path
                .strip_prefix(root.as_std_path())
                .with_context(|| format!("file outside root: {}", path.display()))?;
            let Some(rel) = Utf8PathBuf::from_path_buf(rel.to_path_buf()).ok() else {
                continue;
            };

            let id = files.len() as FileId;
            let text = String::from_utf8_lossy(&bytes);
            for word in search::extract_words(&text) {
                words.entry(word).or_default().insert(id);
            }
            for trigram in search::extract_trigrams(&text) {
                trigrams.entry(trigram).or_default().insert(id);
            }

            let metadata = fs::metadata(&path)
                .with_context(|| format!("failed to stat {}", path.display()))?;
            files.push(FileEntry {
                id,
                path: rel,
                bytes: bytes.len() as u64,
                lines: text.lines().count() as u32,
                mtime_ms: mtime_ms(&metadata),
                hash: blake3::hash(&bytes).to_hex().to_string(),
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            version: 2,
            root,
            files,
            words: flatten(words),
            trigrams: flatten(trigrams),
        })
    }

    pub fn file_by_id(&self, id: FileId) -> Option<&FileEntry> {
        self.files.iter().find(|file| file.id == id)
    }
}

fn flatten(index: BTreeMap<String, BTreeSet<FileId>>) -> BTreeMap<String, Vec<FileId>> {
    index
        .into_iter()
        .map(|(key, ids)| (key, ids.into_iter().collect()))
        .collect()
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
