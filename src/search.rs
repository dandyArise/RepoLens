use std::collections::BTreeSet;
use std::fs;

use anyhow::Result;

use crate::index::{FileId, ProjectIndex};
use crate::pathing::safe_join;

pub fn search(index: &ProjectIndex, query: &str, limit: usize) -> Result<()> {
    let candidates = candidate_files(index, query);
    let needle = query.to_lowercase();
    let mut found = 0usize;

    for id in candidates {
        if found >= limit {
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
                println!("{}:{}: {}", file.path, line_no + 1, line.trim());
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    Ok(())
}

pub fn word(index: &ProjectIndex, word: &str, limit: usize) {
    let key = normalize_word(word);
    let Some(ids) = index.words.get(&key) else {
        return;
    };

    for id in ids.iter().take(limit) {
        if let Some(file) = index.file_by_id(*id) {
            println!("{}", file.path);
        }
    }
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

fn candidate_files(index: &ProjectIndex, query: &str) -> Vec<FileId> {
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
}
