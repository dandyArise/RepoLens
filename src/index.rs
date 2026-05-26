use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::pathing::canonical_utf8;
use crate::scanner;
use crate::search;
use crate::symbols::{self, Symbol};

pub type FileId = u32;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub root: Utf8PathBuf,
    pub files: Vec<FileEntry>,
    pub words: BTreeMap<String, Vec<FileId>>,
    pub trigrams: BTreeMap<String, Vec<FileId>>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: FileId,
    pub path: Utf8PathBuf,
    pub bytes: u64,
    pub lines: u32,
    pub line_offsets: Vec<u64>,
    pub mtime_ms: u64,
    pub hash: String,
}

impl ProjectIndex {
    pub fn build(root: &Path) -> Result<Self> {
        let root = canonical_utf8(root)?;
        let config = Config::load(root.as_std_path())?;
        let mut files = Vec::new();
        let mut words: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();
        let mut trigrams: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();
        let mut symbol_list = Vec::new();

        for path in scanner::source_files(root.as_std_path(), &config)? {
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
            match rel.extension() {
                Some("rs") => symbol_list.extend(symbols::extract_rust_symbols(id, &rel, &text)?),
                Some("js") | Some("mjs") | Some("cjs") => {
                    symbol_list.extend(symbols::extract_javascript_symbols(id, &rel, &text)?);
                }
                Some("ts") | Some("mts") | Some("cts") => {
                    symbol_list
                        .extend(symbols::extract_typescript_symbols(id, &rel, &text, false)?);
                }
                Some("tsx") | Some("jsx") => {
                    symbol_list.extend(symbols::extract_typescript_symbols(id, &rel, &text, true)?);
                }
                _ => {}
            }

            let metadata = fs::metadata(&path)
                .with_context(|| format!("failed to stat {}", path.display()))?;
            files.push(FileEntry {
                id,
                path: rel,
                bytes: bytes.len() as u64,
                lines: text.lines().count() as u32,
                line_offsets: line_offsets(&text),
                mtime_ms: mtime_ms(&metadata),
                hash: blake3::hash(&bytes).to_hex().to_string(),
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            version: 3,
            root,
            files,
            words: flatten(words),
            trigrams: flatten(trigrams),
            symbols: symbol_list,
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

fn line_offsets(text: &str) -> Vec<u64> {
    let mut offsets = vec![0];
    for (idx, byte) in text.bytes().enumerate() {
        if byte == b'\n' && idx + 1 < text.len() {
            offsets.push((idx + 1) as u64);
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::line_offsets;

    #[test]
    fn records_line_offsets() {
        assert_eq!(line_offsets("a\nbc\n"), vec![0, 2]);
        assert_eq!(line_offsets("abc"), vec![0]);
    }
}
