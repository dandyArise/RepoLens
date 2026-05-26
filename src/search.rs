use std::collections::BTreeSet;
use std::fs;

use anyhow::Result;
use serde::Serialize;

use crate::index::{FileId, ProjectIndex};
use crate::pathing::safe_join;

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

pub fn search(index: &ProjectIndex, query: &str, limit: usize) -> Result<()> {
    for hit in search_hits(index, query, limit)? {
        println!("{}:{}: {}", hit.path, hit.line, hit.text);
    }
    Ok(())
}

pub fn search_hits(index: &ProjectIndex, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let candidates = candidate_files(index, query);
    let needle = query.to_lowercase();
    let mut hits = Vec::new();

    for id in candidates {
        if hits.len() >= limit {
            break;
        }
        let Some(file) = index.file_by_id(id) else {
            continue;
        };
        let full_path = safe_join(&index.root, &file.path)?;
        let Ok(content) = fs::read_to_string(&full_path) else {
            continue;
        };

        for (line_no, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                hits.push(SearchHit {
                    path: file.path.to_string(),
                    line: line_no + 1,
                    text: line.trim().to_string(),
                });
                if hits.len() >= limit {
                    break;
                }
            }
        }
    }

    Ok(hits)
}

pub fn word(index: &ProjectIndex, word: &str, limit: usize) {
    for path in word_paths(index, word, limit) {
        println!("{path}");
    }
}

pub fn word_paths(index: &ProjectIndex, word: &str, limit: usize) -> Vec<String> {
    let key = normalize_word(word);
    let Some(ids) = index.words.get(&key) else {
        return Vec::new();
    };

    ids.iter()
        .take(limit)
        .filter_map(|id| index.file_by_id(*id))
        .map(|file| file.path.to_string())
        .collect()
}

pub fn extract_words(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
        .filter(|word| word.len() > 1)
        .map(normalize_word)
        .collect()
}

pub fn extract_trigrams(text: &str) -> BTreeSet<String> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    if bytes.len() < 3 {
        return BTreeSet::new();
    }

    bytes
        .windows(3)
        .filter(|tri| {
            tri.iter()
                .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
        })
        .map(|tri| String::from_utf8_lossy(tri).into_owned())
        .collect()
}

pub(crate) fn candidate_files(index: &ProjectIndex, query: &str) -> Vec<FileId> {
    let trigrams: Vec<_> = extract_trigrams(query).into_iter().collect();
    if trigrams.is_empty() {
        return index.files.iter().map(|file| file.id).collect();
    }

    let mut sets = trigrams
        .iter()
        .filter_map(|trigram| index.trigrams.get(trigram))
        .map(|ids| ids.iter().copied().collect::<BTreeSet<_>>());

    let Some(mut acc) = sets.next() else {
        return Vec::new();
    };

    for set in sets {
        acc = acc.intersection(&set).copied().collect();
        if acc.is_empty() {
            break;
        }
    }

    acc.into_iter().collect()
}

fn normalize_word(word: &str) -> String {
    word.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use camino::Utf8PathBuf;

    use crate::index::{FileEntry, ProjectIndex};

    use super::{extract_trigrams, extract_words};

    #[test]
    fn extracts_identifier_words() {
        let words = extract_words("fn UserService() { user_id }");
        assert!(words.contains("userservice"));
        assert!(words.contains("user_id"));
    }

    #[test]
    fn extracts_query_trigrams() {
        let trigrams = extract_trigrams("abcd");
        assert!(trigrams.contains("abc"));
        assert!(trigrams.contains("bcd"));
    }

    #[test]
    fn intersects_trigram_candidates() {
        let mut trigrams = BTreeMap::new();
        trigrams.insert("abc".to_string(), vec![0, 1]);
        trigrams.insert("bcd".to_string(), vec![1]);
        let index = ProjectIndex {
            version: 2,
            root: Utf8PathBuf::from("."),
            files: vec![file(0, "a.txt"), file(1, "b.txt")],
            words: BTreeMap::new(),
            trigrams,
            symbols: Vec::new(),
            symbols_by_name: BTreeMap::new(),
            deps: Vec::new(),
        };

        assert_eq!(super::candidate_files(&index, "abcd"), vec![1]);
    }

    fn file(id: u32, path: &str) -> FileEntry {
        FileEntry {
            id,
            path: Utf8PathBuf::from(path),
            bytes: 0,
            lines: 0,
            line_offsets: vec![0],
            mtime_ms: 0,
            hash: String::new(),
        }
    }
}
