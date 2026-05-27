use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

use crate::index::{FileEntry, FileId, ProjectIndex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDeps {
    pub file_id: FileId,
    pub path: Utf8PathBuf,
    pub imports: Vec<ImportRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRef {
    pub module: String,
    pub line: usize,
    pub kind: ImportKind,
    #[serde(default)]
    pub resolved_path: Option<Utf8PathBuf>,
    #[serde(default)]
    pub resolved_file_id: Option<FileId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    RustUse,
    RustMod,
    EsImport,
    CommonJsRequire,
    PythonImport,
    PythonFromImport,
    GoImport,
    PhpUse,
    PhpRequire,
}

pub fn print_deps(index: &ProjectIndex, path: &Utf8PathBuf) {
    for deps in deps_for_file(index, path) {
        for import in deps.imports {
            if let Some(resolved) = import.resolved_path {
                println!(
                    "{}:{} {:?} {} -> {}",
                    deps.path, import.line, import.kind, import.module, resolved
                );
            } else {
                println!(
                    "{}:{} {:?} {}",
                    deps.path, import.line, import.kind, import.module
                );
            }
        }
    }
}

pub fn deps_for_file(index: &ProjectIndex, path: &Utf8PathBuf) -> Vec<FileDeps> {
    index
        .deps
        .iter()
        .filter(|deps| deps.path == *path)
        .cloned()
        .collect()
}

pub fn print_reverse_deps(index: &ProjectIndex, path: &Utf8PathBuf) {
    for path in reverse_deps_for_file(index, path) {
        println!("{path}");
    }
}

pub fn reverse_deps_for_file(index: &ProjectIndex, path: &Utf8PathBuf) -> Vec<Utf8PathBuf> {
    let Some(file) = index.file_by_path(path) else {
        return Vec::new();
    };
    index
        .deps_reverse
        .get(&file.id)
        .into_iter()
        .flatten()
        .filter_map(|id| index.file_by_id(*id))
        .map(|file| file.path.clone())
        .collect()
}

pub fn build_graph(
    deps: &[FileDeps],
) -> (BTreeMap<FileId, Vec<FileId>>, BTreeMap<FileId, Vec<FileId>>) {
    let mut forward: BTreeMap<FileId, BTreeSet<FileId>> = BTreeMap::new();
    let mut reverse: BTreeMap<FileId, BTreeSet<FileId>> = BTreeMap::new();

    for file_deps in deps {
        for target in file_deps
            .imports
            .iter()
            .filter_map(|import| import.resolved_file_id)
        {
            forward.entry(file_deps.file_id).or_default().insert(target);
            reverse.entry(target).or_default().insert(file_deps.file_id);
        }
    }

    (flatten_graph(forward), flatten_graph(reverse))
}

pub fn resolve_relative_ts_js_imports(deps: &mut [FileDeps], files: &[FileEntry]) {
    let by_path: BTreeMap<_, _> = files
        .iter()
        .map(|file| (file.path.clone(), file.id))
        .collect();

    for file_deps in deps {
        for import in &mut file_deps.imports {
            if !matches!(
                import.kind,
                ImportKind::EsImport | ImportKind::CommonJsRequire
            ) || !is_relative_specifier(&import.module)
            {
                continue;
            }

            if let Some((path, id)) =
                resolve_ts_js_specifier(&file_deps.path, &import.module, &by_path)
            {
                import.resolved_path = Some(path);
                import.resolved_file_id = Some(id);
            }
        }
    }
}

fn flatten_graph(graph: BTreeMap<FileId, BTreeSet<FileId>>) -> BTreeMap<FileId, Vec<FileId>> {
    graph
        .into_iter()
        .map(|(source, targets)| (source, targets.into_iter().collect()))
        .collect()
}

pub fn extract_rust_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("failed to load Rust tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Rust file"))?;
    let mut imports = Vec::new();
    walk_rust_deps(tree.root_node(), text.as_bytes(), &mut imports);
    Ok(FileDeps {
        file_id,
        path: path.clone(),
        imports,
    })
}

pub fn extract_ts_like_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .context("failed to load TypeScript tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse TS/JS file"))?;
    let mut imports = Vec::new();
    walk_ts_like_deps(tree.root_node(), text.as_bytes(), &mut imports);
    Ok(FileDeps {
        file_id,
        path: path.clone(),
        imports,
    })
}

pub fn extract_python_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .context("failed to load Python tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Python file"))?;
    let mut imports = Vec::new();
    walk_python_deps(tree.root_node(), text.as_bytes(), &mut imports);
    Ok(FileDeps {
        file_id,
        path: path.clone(),
        imports,
    })
}

pub fn extract_go_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("failed to load Go tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Go file"))?;
    let mut imports = Vec::new();
    walk_go_deps(tree.root_node(), text.as_bytes(), &mut imports);
    Ok(FileDeps {
        file_id,
        path: path.clone(),
        imports,
    })
}

pub fn extract_php_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .context("failed to load PHP tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse PHP file"))?;
    let mut imports = Vec::new();
    walk_php_deps(tree.root_node(), text.as_bytes(), &mut imports);
    Ok(FileDeps {
        file_id,
        path: path.clone(),
        imports,
    })
}

fn walk_rust_deps(node: Node, source: &[u8], imports: &mut Vec<ImportRef>) {
    match node.kind() {
        "use_declaration" => {
            if let Ok(text) = node.utf8_text(source) {
                imports.push(import(node, clean_rust_use(text), ImportKind::RustUse));
            }
        }
        "mod_item" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                imports.push(import(node, name.to_string(), ImportKind::RustMod));
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_deps(child, source, imports);
    }
}

fn walk_ts_like_deps(node: Node, source: &[u8], imports: &mut Vec<ImportRef>) {
    match node.kind() {
        "import_statement" => {
            if let Some(module) = string_child(node, source) {
                imports.push(import(node, module, ImportKind::EsImport));
            }
        }
        "call_expression" if call_name(node, source).as_deref() == Some("require") => {
            if let Some(module) = string_child(node, source) {
                imports.push(import(node, module, ImportKind::CommonJsRequire));
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ts_like_deps(child, source, imports);
    }
}

fn walk_python_deps(node: Node, source: &[u8], imports: &mut Vec<ImportRef>) {
    match node.kind() {
        "import_statement" => {
            if let Ok(text) = node.utf8_text(source) {
                imports.push(import(
                    node,
                    text.trim_start_matches("import").trim().to_string(),
                    ImportKind::PythonImport,
                ));
            }
        }
        "import_from_statement" => {
            if let Ok(text) = node.utf8_text(source) {
                imports.push(import(
                    node,
                    text.trim_start_matches("from").trim().to_string(),
                    ImportKind::PythonFromImport,
                ));
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python_deps(child, source, imports);
    }
}

fn walk_go_deps(node: Node, source: &[u8], imports: &mut Vec<ImportRef>) {
    if node.kind() == "import_declaration" {
        collect_string_literals(node, source, ImportKind::GoImport, imports);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go_deps(child, source, imports);
    }
}

fn walk_php_deps(node: Node, source: &[u8], imports: &mut Vec<ImportRef>) {
    match node.kind() {
        "namespace_use_declaration" => {
            if let Ok(text) = node.utf8_text(source) {
                imports.push(import(
                    node,
                    text.trim_start_matches("use")
                        .trim_end_matches(';')
                        .trim()
                        .to_string(),
                    ImportKind::PhpUse,
                ));
            }
        }
        "require_expression"
        | "include_expression"
        | "require_once_expression"
        | "include_once_expression" => {
            collect_string_literals(node, source, ImportKind::PhpRequire, imports);
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_deps(child, source, imports);
    }
}

fn import(node: Node, module: String, kind: ImportKind) -> ImportRef {
    ImportRef {
        module,
        line: node.start_position().row + 1,
        kind,
        resolved_path: None,
        resolved_file_id: None,
    }
}

fn is_relative_specifier(module: &str) -> bool {
    module.starts_with("./") || module.starts_with("../")
}

fn resolve_ts_js_specifier(
    from: &Utf8Path,
    module: &str,
    by_path: &BTreeMap<Utf8PathBuf, FileId>,
) -> Option<(Utf8PathBuf, FileId)> {
    let base = from.parent().unwrap_or_else(|| Utf8Path::new(""));
    let requested = normalize_relative_path(&base.join(module));
    for candidate in ts_js_candidates(&requested) {
        if let Some(id) = by_path.get(&candidate) {
            return Some((candidate, *id));
        }
    }
    None
}

fn ts_js_candidates(path: &Utf8Path) -> Vec<Utf8PathBuf> {
    const EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];

    if path.extension().is_some() {
        return vec![path.to_path_buf()];
    }

    let mut candidates = vec![path.to_path_buf()];
    for ext in EXTENSIONS {
        let mut candidate = path.to_path_buf();
        candidate.set_extension(ext);
        candidates.push(candidate);
    }
    for ext in EXTENSIONS {
        candidates.push(path.join(format!("index.{ext}")));
    }
    candidates
}

fn normalize_relative_path(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                out.pop();
            }
            Utf8Component::Normal(part) => out.push(part),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => return path.to_path_buf(),
        }
    }
    out
}

fn clean_rust_use(text: &str) -> String {
    text.trim()
        .trim_start_matches("use")
        .trim()
        .trim_end_matches(';')
        .to_string()
}

fn string_child(node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            return Some(clean_string(child.utf8_text(source).ok()?));
        }
        if let Some(found) = string_child(child, source) {
            return Some(found);
        }
    }
    None
}

fn call_name(node: Node, source: &[u8]) -> Option<String> {
    node.child_by_field_name("function")?
        .utf8_text(source)
        .ok()
        .map(str::to_string)
}

fn clean_string(text: &str) -> String {
    text.trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .to_string()
}

fn collect_string_literals(
    node: Node,
    source: &[u8],
    kind: ImportKind,
    imports: &mut Vec<ImportRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "interpreted_string_literal" | "raw_string_literal" | "string" | "string_literal"
        ) && let Ok(text) = child.utf8_text(source)
        {
            imports.push(import(child, clean_string(text), kind.clone()));
        }
        collect_string_literals(child, source, kind.clone(), imports);
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use crate::index::FileEntry;

    use super::{
        FileDeps, ImportKind, ImportRef, build_graph, extract_go_deps, extract_php_deps,
        extract_python_deps, extract_rust_deps, extract_ts_like_deps,
        resolve_relative_ts_js_imports,
    };

    #[test]
    fn extracts_rust_deps() {
        let deps = extract_rust_deps(
            0,
            &Utf8PathBuf::from("main.rs"),
            "use crate::config::Config;\nmod scanner;\n",
        )
        .unwrap();

        assert!(
            deps.imports
                .iter()
                .any(|i| i.module == "crate::config::Config")
        );
        assert!(deps.imports.iter().any(|i| i.module == "scanner"));
    }

    #[test]
    fn extracts_ts_deps() {
        let deps = extract_ts_like_deps(
            0,
            &Utf8PathBuf::from("main.ts"),
            "import x from './x';\nconst y = require(\"./y\");\n",
        )
        .unwrap();

        assert!(
            deps.imports
                .iter()
                .any(|i| i.module == "./x" && i.kind == ImportKind::EsImport)
        );
        assert!(
            deps.imports
                .iter()
                .any(|i| i.module == "./y" && i.kind == ImportKind::CommonJsRequire)
        );
    }

    #[test]
    fn extracts_python_deps() {
        let deps = extract_python_deps(
            0,
            &Utf8PathBuf::from("main.py"),
            "import os\nfrom pathlib import Path\n",
        )
        .unwrap();

        assert!(deps.imports.iter().any(|i| i.module == "os"));
        assert!(
            deps.imports
                .iter()
                .any(|i| i.module == "pathlib import Path")
        );
    }

    #[test]
    fn extracts_go_deps() {
        let deps = extract_go_deps(
            0,
            &Utf8PathBuf::from("main.go"),
            "package main\nimport (\n \"fmt\"\n alias \"example.com/app\"\n)\n",
        )
        .unwrap();

        assert!(deps.imports.iter().any(|i| i.module == "fmt"));
        assert!(deps.imports.iter().any(|i| i.module == "example.com/app"));
    }

    #[test]
    fn extracts_php_deps() {
        let deps = extract_php_deps(
            0,
            &Utf8PathBuf::from("main.php"),
            "<?php\nuse App\\Service;\nrequire 'vendor/autoload.php';\n",
        )
        .unwrap();

        assert!(deps.imports.iter().any(|i| i.module == "App\\Service"));
        assert!(
            deps.imports
                .iter()
                .any(|i| i.module == "vendor/autoload.php")
        );
    }

    #[test]
    fn resolves_relative_ts_imports_to_indexed_files() {
        let files = vec![
            file(0, "src/main.ts"),
            file(1, "src/utils/index.ts"),
            file(2, "src/lib/math.ts"),
        ];
        let mut deps = vec![FileDeps {
            file_id: 0,
            path: Utf8PathBuf::from("src/main.ts"),
            imports: vec![
                import("./utils", ImportKind::EsImport),
                import("./lib/math", ImportKind::CommonJsRequire),
                import("react", ImportKind::EsImport),
            ],
        }];

        resolve_relative_ts_js_imports(&mut deps, &files);

        assert_eq!(
            deps[0].imports[0].resolved_path,
            Some(Utf8PathBuf::from("src/utils/index.ts"))
        );
        assert_eq!(deps[0].imports[0].resolved_file_id, Some(1));
        assert_eq!(
            deps[0].imports[1].resolved_path,
            Some(Utf8PathBuf::from("src/lib/math.ts"))
        );
        assert_eq!(deps[0].imports[2].resolved_path, None);
    }

    #[test]
    fn builds_forward_and_reverse_dependency_graphs() {
        let deps = vec![FileDeps {
            file_id: 0,
            path: Utf8PathBuf::from("src/main.ts"),
            imports: vec![ImportRef {
                module: "./util".to_string(),
                line: 1,
                kind: ImportKind::EsImport,
                resolved_path: Some(Utf8PathBuf::from("src/util.ts")),
                resolved_file_id: Some(1),
            }],
        }];

        let (forward, reverse) = build_graph(&deps);

        assert_eq!(forward.get(&0), Some(&vec![1]));
        assert_eq!(reverse.get(&1), Some(&vec![0]));
    }

    fn file(id: u32, path: &str) -> FileEntry {
        FileEntry {
            id,
            path: Utf8PathBuf::from(path),
            bytes: 1,
            lines: 1,
            line_offsets: vec![0],
            mtime_ms: 0,
            hash: String::new(),
        }
    }

    fn import(module: &str, kind: ImportKind) -> ImportRef {
        ImportRef {
            module: module.to_string(),
            line: 1,
            kind,
            resolved_path: None,
            resolved_file_id: None,
        }
    }
}
