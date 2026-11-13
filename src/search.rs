use std::collections::BTreeSet;
use std::fs;

use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;

use crate::index::{FileId, ProjectIndex};
use crate::pathing::safe_join;
use crate::{config::Config, security};

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
    search_hits_impl(index, query, limit, false)
}

// Persistent servers amortize worker-pool startup over requests. One-shot CLI
// searches stay sequential rather than paying that startup cost every time.
pub fn search_hits_parallel(
    index: &ProjectIndex,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    search_hits_impl(index, query, limit, true)
}

fn search_hits_impl(
    index: &ProjectIndex,
    query: &str,
    limit: usize,
    parallel: bool,
) -> Result<Vec<SearchHit>> {
    let config = Config::load(index.root.as_std_path())?;
    let candidates = candidate_files(index, query);
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    // Keep short interactive searches lazy. Large result sets can scan files
    // independently, then merge in candidate order to preserve global limits
    // and which errors are observable before that limit is reached.
    let parallel_work = parallel
        && limit >= 1024
        && candidates.len() >= 8
        && candidates
            .iter()
            .filter_map(|&id| index.file_by_id(id))
            .map(|file| file.bytes)
            .sum::<u64>()
            >= 256 * 1024;
    if parallel_work {
        let batches: Vec<_> = candidates
            .par_iter()
            .map(|&id| search_file(index, id, &config, &needle, limit))
            .collect();
        for batch in batches {
            if hits.len() >= limit {
                break;
            }
            hits.extend(batch?.into_iter().take(limit - hits.len()));
        }
        return Ok(hits);
    }
    for id in candidates {
        if hits.len() >= limit {
            break;
        }
        hits.extend(search_file(
            index,
            id,
            &config,
            &needle,
            limit - hits.len(),
        )?);
    }
    Ok(hits)
}

fn search_file(
    index: &ProjectIndex,
    id: FileId,
    config: &Config,
    needle: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    let Some(file) = index.file_by_id(id) else {
        return Ok(hits);
    };
    let full_path = safe_join(&index.root, &file.path)?;
    if !config.allow_sensitive && security::is_sensitive_path(full_path.as_std_path()) {
        return Ok(hits);
    }
    let Ok(content) = fs::read_to_string(&full_path) else {
        return Ok(hits);
    };
    let mut scratch = String::new();
    for (line_no, line) in content.lines().enumerate() {
        let matches = if line.is_ascii() {
            scratch.clear();
            scratch.push_str(line);
            scratch.make_ascii_lowercase();
            scratch.contains(needle)
        } else {
            // Preserve Rust's Unicode lowercase semantics, including
            // context-sensitive mappings and length-changing mappings.
            line.to_lowercase().contains(needle)
        };
        if matches {
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

    if trigrams
        .iter()
        .any(|trigram| !index.trigrams.contains_key(trigram))
    {
        return Vec::new();
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
    fn optimized_search_matches_serial_reference_with_limits_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let content = "  ToRcH DEF sample  \r\n".repeat(1200)
            + "TORCH ΟΣ İ K é 🦀\nfinal torch without newline";
        for i in 0..12 {
            std::fs::write(dir.path().join(format!("file_{i:02}.rs")), &content).unwrap();
        }
        let index = ProjectIndex::build(dir.path()).unwrap();
        // The search must still read current source, not stale cached content.
        std::fs::write(index.root.join(&index.files[0].path), "edited torch\n").unwrap();
        std::fs::remove_file(index.root.join(&index.files[1].path)).unwrap();
        for query in ["torch", "def ", "ΟΣ", "i\u{307}", "k", "", "absent"] {
            for limit in [0, 1, 1023, 1024, usize::MAX] {
                let mut expected = Vec::new();
                for id in super::candidate_files(&index, query) {
                    let file = index.file_by_id(id).unwrap();
                    let Ok(text) = std::fs::read_to_string(index.root.join(&file.path)) else {
                        continue;
                    };
                    for (line, text) in text.lines().enumerate() {
                        if text.to_lowercase().contains(&query.to_lowercase()) {
                            expected.push((
                                file.path.to_string(),
                                line + 1,
                                text.trim().to_owned(),
                            ));
                        }
                    }
                }
                expected.truncate(limit);
                let actual: Vec<_> = super::search_hits_parallel(&index, query, limit)
                    .unwrap()
                    .into_iter()
                    .map(|hit| (hit.path, hit.line, hit.text))
                    .collect();
                assert_eq!(actual, expected, "query={query:?}, limit={limit}");
            }
        }
    }

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
    fn sensitive_search_rechecks_policy_for_stale_indexes() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join(".repolensrc.toml");
        std::fs::write(&config, "allow_sensitive = true\ninclude_hidden = true\n").unwrap();
        std::fs::write(temp.path().join(".env"), "SYNTHETIC_MARKER=example\n").unwrap();
        let index = ProjectIndex::build(temp.path()).unwrap();
        assert_eq!(
            super::search_hits(&index, "SYNTHETIC_MARKER", 10)
                .unwrap()
                .len(),
            1
        );
        std::fs::write(&config, "allow_sensitive = false\ninclude_hidden = true\n").unwrap();
        assert!(
            super::search_hits(&index, "SYNTHETIC_MARKER", 10)
                .unwrap()
                .is_empty()
        );
        std::fs::write(&config, "not valid toml [").unwrap();
        assert!(super::search_hits(&index, "SYNTHETIC_MARKER", 10).is_err());
    }

    #[test]
    fn intersects_trigram_candidates() {
        let mut trigrams = BTreeMap::new();
        trigrams.insert("abc".to_string(), vec![0, 1]);
        trigrams.insert("bcd".to_string(), vec![1]);
        let index = ProjectIndex {
            indexed_config: crate::config::Config::default(),
            version: 2,
            root: Utf8PathBuf::from("."),
            files: vec![file(0, "a.txt"), file(1, "b.txt")],
            words: BTreeMap::new(),
            trigrams,
            symbols: Vec::new(),
            symbols_by_name: BTreeMap::new(),
            deps: Vec::new(),
            deps_forward: BTreeMap::new(),
            deps_reverse: BTreeMap::new(),
        };

        assert_eq!(super::candidate_files(&index, "abcd"), vec![1]);
        assert!(super::candidate_files(&index, "abcde").is_empty());
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
