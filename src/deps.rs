use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

use crate::index::{FileId, ProjectIndex};

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
}

pub fn print_deps(index: &ProjectIndex, path: &Utf8PathBuf) {
    for deps in deps_for_file(index, path) {
        for import in deps.imports {
            println!(
                "{}:{} {:?} {}",
                deps.path, import.line, import.kind, import.module
            );
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

fn import(node: Node, module: String, kind: ImportKind) -> ImportRef {
    ImportRef {
        module,
        line: node.start_position().row + 1,
        kind,
    }
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
    text.trim_matches('"').trim_matches('\'').to_string()
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::{ImportKind, extract_python_deps, extract_rust_deps, extract_ts_like_deps};

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
}
