use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::deps::{self, FileDeps};
use crate::pathing::canonical_utf8;
use crate::scanner;
use crate::search;
use crate::symbols::{self, Symbol};

pub type FileId = u32;
pub type SymbolId = u32;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub root: Utf8PathBuf,
    pub files: Vec<FileEntry>,
    pub words: BTreeMap<String, Vec<FileId>>,
    pub trigrams: BTreeMap<String, Vec<FileId>>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub symbols_by_name: BTreeMap<String, Vec<SymbolId>>,
    #[serde(default)]
    pub deps: Vec<FileDeps>,
    #[serde(default)]
    pub deps_forward: BTreeMap<FileId, Vec<FileId>>,
    #[serde(default)]
    pub deps_reverse: BTreeMap<FileId, Vec<FileId>>,
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

struct IndexedFile {
    entry: FileEntry,
    words: BTreeSet<String>,
    trigrams: BTreeSet<String>,
    symbols: Vec<Symbol>,
    deps: Option<FileDeps>,
}

impl ProjectIndex {
    pub fn build(root: &Path) -> Result<Self> {
        let root = canonical_utf8(root)?;
        let config = Config::load(root.as_std_path())?;
        let mut files = Vec::new();
        let mut words: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();
        let mut trigrams: BTreeMap<String, BTreeSet<FileId>> = BTreeMap::new();
        let mut symbol_list = Vec::new();
        let mut deps_list = Vec::new();

        let indexed = scanner::source_files(root.as_std_path(), &config)?
            .into_par_iter()
            .enumerate()
            .map(|(id, path)| index_source_file(&root, id as FileId, &path))
            .collect::<Result<Vec<_>>>()?;

        for indexed in indexed.into_iter().flatten() {
            let id = indexed.entry.id;
            for word in indexed.words {
                words.entry(word).or_default().insert(id);
            }
            for trigram in indexed.trigrams {
                trigrams.entry(trigram).or_default().insert(id);
            }
            symbol_list.extend(indexed.symbols);
            if let Some(deps) = indexed.deps {
                deps_list.push(deps);
            }
            files.push(indexed.entry);
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        deps::resolve_relative_ts_js_imports(&mut deps_list, &files);
        let (deps_forward, deps_reverse) = deps::build_graph(&deps_list);
        let symbols_by_name = index_symbols_by_name(&symbol_list);
        Ok(Self {
            version: 6,
            root,
            files,
            words: flatten(words),
            trigrams: flatten(trigrams),
            symbols: symbol_list,
            symbols_by_name,
            deps: deps_list,
            deps_forward,
            deps_reverse,
        })
    }

    pub fn file_by_id(&self, id: FileId) -> Option<&FileEntry> {
        self.files.iter().find(|file| file.id == id)
    }

    pub fn file_by_path(&self, path: &Utf8PathBuf) -> Option<&FileEntry> {
        self.files.iter().find(|file| file.path == *path)
    }

    pub fn refresh_file(&mut self, path: &Utf8PathBuf) -> Result<()> {
        self.upsert_file(path)
    }

    pub fn upsert_file(&mut self, path: &Utf8PathBuf) -> Result<()> {
        let id = self
            .file_by_path(path)
            .map(|file| file.id)
            .unwrap_or_else(|| self.files.iter().map(|file| file.id).max().unwrap_or(0) + 1);
        let full_path = self.root.join(path);
        let bytes =
            fs::read(&full_path).with_context(|| format!("failed to read {}", full_path))?;
        if scanner::looks_binary(&bytes) {
            *self = Self::build(self.root.as_std_path())?;
            return Ok(());
        }

        let text = String::from_utf8_lossy(&bytes);
        let metadata =
            fs::metadata(&full_path).with_context(|| format!("failed to stat {path}"))?;
        let next_file = FileEntry {
            id,
            path: path.clone(),
            bytes: bytes.len() as u64,
            lines: text.lines().count() as u32,
            line_offsets: line_offsets(&text),
            mtime_ms: mtime_ms(&metadata),
            hash: blake3::hash(&bytes).to_hex().to_string(),
        };

        remove_file_id(&mut self.words, id);
        remove_file_id(&mut self.trigrams, id);
        self.symbols.retain(|symbol| symbol.file_id != id);
        self.deps.retain(|deps| deps.file_id != id);

        for word in search::extract_words(&text) {
            push_unique(self.words.entry(word).or_default(), id);
        }
        for trigram in search::extract_trigrams(&text) {
            push_unique(self.trigrams.entry(trigram).or_default(), id);
        }
        self.symbols.extend(extract_symbols(id, path, &text)?);
        if let Some(file_deps) = extract_deps(id, path, &text)? {
            self.deps.push(file_deps);
        }

        if let Some(file) = self.files.iter_mut().find(|file| file.id == id) {
            *file = next_file;
        } else {
            self.files.push(next_file);
        }
        self.rebuild_derived_indexes();
        Ok(())
    }

    pub fn remove_file(&mut self, path: &Utf8PathBuf) -> Result<()> {
        let Some(file) = self.file_by_path(path) else {
            return Ok(());
        };
        let id = file.id;
        self.files.retain(|file| file.id != id);
        remove_file_id(&mut self.words, id);
        remove_file_id(&mut self.trigrams, id);
        self.symbols.retain(|symbol| symbol.file_id != id);
        self.deps.retain(|deps| deps.file_id != id);
        for deps in &mut self.deps {
            for import in &mut deps.imports {
                if import.resolved_file_id == Some(id) {
                    import.resolved_file_id = None;
                    import.resolved_path = None;
                }
            }
        }
        self.rebuild_derived_indexes();
        Ok(())
    }

    fn rebuild_derived_indexes(&mut self) {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        deps::resolve_relative_ts_js_imports(&mut self.deps, &self.files);
        let (deps_forward, deps_reverse) = deps::build_graph(&self.deps);
        self.deps_forward = deps_forward;
        self.deps_reverse = deps_reverse;
        self.symbols_by_name = index_symbols_by_name(&self.symbols);
    }
}

fn extract_symbols(id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Vec<Symbol>> {
    match path.extension() {
        Some("rs") => symbols::extract_rust_symbols(id, path, text),
        Some("js") | Some("mjs") | Some("cjs") => {
            symbols::extract_javascript_symbols(id, path, text)
        }
        Some("ts") | Some("mts") | Some("cts") => {
            symbols::extract_typescript_symbols(id, path, text, false)
        }
        Some("tsx") | Some("jsx") => symbols::extract_typescript_symbols(id, path, text, true),
        Some("py") | Some("pyw") => symbols::extract_python_symbols(id, path, text),
        Some("go") => symbols::extract_go_symbols(id, path, text),
        Some("php") => symbols::extract_php_symbols(id, path, text),
        Some("java") => symbols::extract_java_symbols(id, path, text),
        Some("cs") => symbols::extract_c_sharp_symbols(id, path, text),
        Some("c") | Some("h") => symbols::extract_c_symbols(id, path, text),
        Some("cc") | Some("cpp") | Some("cxx") | Some("hpp") | Some("hh") | Some("hxx") => {
            symbols::extract_cpp_symbols(id, path, text)
        }
        Some("rb") => symbols::extract_ruby_symbols(id, path, text),
        Some("json") => symbols::extract_json_symbols(id, path, text),
        Some("toml") => symbols::extract_toml_symbols(id, path, text),
        Some("yml") | Some("yaml") => Ok(symbols::extract_yaml_symbols(id, path, text)),
        _ => Ok(Vec::new()),
    }
}

fn index_source_file(root: &Utf8PathBuf, id: FileId, path: &Path) -> Result<Option<IndexedFile>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if scanner::looks_binary(&bytes) {
        return Ok(None);
    }

    let rel = path
        .strip_prefix(root.as_std_path())
        .with_context(|| format!("file outside root: {}", path.display()))?;
    let Some(rel) = Utf8PathBuf::from_path_buf(rel.to_path_buf()).ok() else {
        return Ok(None);
    };

    let text = String::from_utf8_lossy(&bytes);
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let symbols = extract_symbols(id, &rel, &text)?;
    let deps = extract_deps(id, &rel, &text)?;

    Ok(Some(IndexedFile {
        entry: FileEntry {
            id,
            path: rel,
            bytes: bytes.len() as u64,
            lines: text.lines().count() as u32,
            line_offsets: line_offsets(&text),
            mtime_ms: mtime_ms(&metadata),
            hash: blake3::hash(&bytes).to_hex().to_string(),
        },
        words: search::extract_words(&text),
        trigrams: search::extract_trigrams(&text),
        symbols,
        deps,
    }))
}

fn extract_deps(id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Option<FileDeps>> {
    match path.extension() {
        Some("rs") => deps::extract_rust_deps(id, path, text).map(Some),
        Some("js") | Some("mjs") | Some("cjs") | Some("ts") | Some("mts") | Some("cts")
        | Some("tsx") | Some("jsx") => deps::extract_ts_like_deps(id, path, text).map(Some),
        Some("py") | Some("pyw") => deps::extract_python_deps(id, path, text).map(Some),
        Some("go") => deps::extract_go_deps(id, path, text).map(Some),
        Some("php") => deps::extract_php_deps(id, path, text).map(Some),
        Some("java") => deps::extract_java_deps(id, path, text).map(Some),
        Some("cs") => deps::extract_c_sharp_deps(id, path, text).map(Some),
        Some("c") | Some("h") | Some("cc") | Some("cpp") | Some("cxx") | Some("hpp")
        | Some("hh") | Some("hxx") => deps::extract_c_like_deps(id, path, text).map(Some),
        Some("rb") => deps::extract_ruby_deps(id, path, text).map(Some),
        _ => Ok(None),
    }
}

fn remove_file_id(index: &mut BTreeMap<String, Vec<FileId>>, id: FileId) {
    index.retain(|_, ids| {
        ids.retain(|existing| *existing != id);
        !ids.is_empty()
    });
}

fn push_unique(ids: &mut Vec<FileId>, id: FileId) {
    if !ids.contains(&id) {
        ids.push(id);
        ids.sort_unstable();
    }
}

fn index_symbols_by_name(symbols: &[Symbol]) -> BTreeMap<String, Vec<SymbolId>> {
    let mut index: BTreeMap<String, Vec<SymbolId>> = BTreeMap::new();
    for (id, symbol) in symbols.iter().enumerate() {
        index
            .entry(symbol.name.to_ascii_lowercase())
            .or_default()
            .push(id as SymbolId);
    }
    index
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
    use std::fs;

    use camino::Utf8PathBuf;

    use crate::symbols::{Symbol, SymbolKind};

    use super::{ProjectIndex, index_symbols_by_name, line_offsets};

    #[test]
    fn records_line_offsets() {
        assert_eq!(line_offsets("a\nbc\n"), vec![0, 2]);
        assert_eq!(line_offsets("abc"), vec![0]);
    }

    #[test]
    fn indexes_symbols_by_normalized_name() {
        let symbols = vec![Symbol {
            name: "ProjectIndex".to_string(),
            kind: SymbolKind::Struct,
            file_id: 0,
            path: Utf8PathBuf::from("src/index.rs"),
            line: 1,
            column: 1,
            end_line: 1,
        }];

        let index = index_symbols_by_name(&symbols);
        assert_eq!(index.get("projectindex").unwrap(), &vec![0]);
    }

    #[test]
    fn indexes_rust_fixture_symbols_and_deps() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src").join("main.rs"),
            "mod scanner;\nuse crate::scanner::Scanner;\nfn main() {}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("src").join("scanner.rs"),
            "pub struct Scanner;\n",
        )
        .unwrap();

        let index = ProjectIndex::build(temp.path()).unwrap();

        assert!(index.files.iter().any(|file| file.path == "src/main.rs"));
        assert!(index.symbols.iter().any(|symbol| symbol.name == "main"));
        assert!(
            index
                .deps
                .iter()
                .flat_map(|deps| deps.imports.iter())
                .any(|import| import.module == "scanner")
        );
    }

    #[test]
    fn indexes_typescript_fixture_relative_deps() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::create_dir(temp.path().join("src").join("utils")).unwrap();
        fs::write(
            temp.path().join("src").join("app.ts"),
            "import { helper } from './utils';\nexport function app() { return helper(); }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("src").join("utils").join("index.ts"),
            "export function helper() { return 1; }\n",
        )
        .unwrap();

        let index = ProjectIndex::build(temp.path()).unwrap();
        let app = index
            .file_by_path(&Utf8PathBuf::from("src/app.ts"))
            .unwrap()
            .id;
        let util = index
            .file_by_path(&Utf8PathBuf::from("src/utils/index.ts"))
            .unwrap()
            .id;

        assert_eq!(index.deps_forward.get(&app), Some(&vec![util]));
        assert_eq!(index.deps_reverse.get(&util), Some(&vec![app]));
    }

    #[test]
    fn refreshes_one_file_indexes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn old_name() {}\n").unwrap();
        fs::write(temp.path().join("other.rs"), "fn untouched() {}\n").unwrap();
        let mut index = ProjectIndex::build(temp.path()).unwrap();
        let main_id = index
            .file_by_path(&Utf8PathBuf::from("main.rs"))
            .unwrap()
            .id;

        fs::write(temp.path().join("main.rs"), "fn new_name() {}\n").unwrap();
        index.refresh_file(&Utf8PathBuf::from("main.rs")).unwrap();

        assert_eq!(
            index
                .file_by_path(&Utf8PathBuf::from("main.rs"))
                .unwrap()
                .id,
            main_id
        );
        assert!(index.symbols.iter().any(|symbol| symbol.name == "new_name"));
        assert!(!index.symbols.iter().any(|symbol| symbol.name == "old_name"));
        assert!(index.words.contains_key("new_name"));
        assert!(!index.words.contains_key("old_name"));
        assert!(
            index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "untouched")
        );
    }

    #[test]
    fn upserts_and_removes_files_incrementally() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut index = ProjectIndex::build(temp.path()).unwrap();

        fs::write(temp.path().join("new.rs"), "fn added() {}\n").unwrap();
        index.upsert_file(&Utf8PathBuf::from("new.rs")).unwrap();

        assert!(index.file_by_path(&Utf8PathBuf::from("new.rs")).is_some());
        assert!(index.symbols.iter().any(|symbol| symbol.name == "added"));

        index.remove_file(&Utf8PathBuf::from("new.rs")).unwrap();

        assert!(index.file_by_path(&Utf8PathBuf::from("new.rs")).is_none());
        assert!(!index.symbols.iter().any(|symbol| symbol.name == "added"));
    }
}
