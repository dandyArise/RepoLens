use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

use crate::index::{FileId, ProjectIndex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_id: FileId,
    pub path: Utf8PathBuf,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Type,
    Const,
    Module,
}

pub fn extract_rust_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("failed to load Rust tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Rust file"))?;

    let mut symbols = Vec::new();
    walk_rust(
        tree.root_node(),
        text.as_bytes(),
        file_id,
        path,
        false,
        &mut symbols,
    );
    Ok(symbols)
}

pub fn print_outline(index: &ProjectIndex, path: &Utf8PathBuf) {
    for symbol in index.symbols.iter().filter(|symbol| symbol.path == *path) {
        println!(
            "{}:{}:{} {:?} {}",
            symbol.path, symbol.line, symbol.column, symbol.kind, symbol.name
        );
    }
}

pub fn print_symbols(index: &ProjectIndex, name: &str, limit: usize) {
    let needle = name.to_ascii_lowercase();
    for symbol in index
        .symbols
        .iter()
        .filter(|symbol| symbol.name.to_ascii_lowercase().contains(&needle))
        .take(limit)
    {
        println!(
            "{}:{}:{} {:?} {}",
            symbol.path, symbol.line, symbol.column, symbol.kind, symbol.name
        );
    }
}

pub fn outline(index: &ProjectIndex, path: &Utf8PathBuf) -> Vec<Symbol> {
    index
        .symbols
        .iter()
        .filter(|symbol| symbol.path == *path)
        .cloned()
        .collect()
}

pub fn find(index: &ProjectIndex, name: &str, limit: usize) -> Vec<Symbol> {
    let needle = name.to_ascii_lowercase();
    index
        .symbols
        .iter()
        .filter(|symbol| symbol.name.to_ascii_lowercase().contains(&needle))
        .take(limit)
        .cloned()
        .collect()
}

fn walk_rust(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_impl: bool,
    symbols: &mut Vec<Symbol>,
) {
    if let Some(symbol) = symbol_from_node(node, source, file_id, path, in_impl) {
        symbols.push(symbol);
    }

    let next_in_impl = in_impl || node.kind() == "impl_item";
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust(child, source, file_id, path, next_in_impl, symbols);
    }
}

fn symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_impl: bool,
) -> Option<Symbol> {
    let kind = match node.kind() {
        "function_item" => {
            if in_impl {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        }
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "impl_item" => SymbolKind::Impl,
        "type_item" => SymbolKind::Type,
        "const_item" => SymbolKind::Const,
        "mod_item" => SymbolKind::Module,
        _ => return None,
    };

    let name = match kind {
        SymbolKind::Impl => impl_name(node, source)?,
        _ => node
            .child_by_field_name("name")?
            .utf8_text(source)
            .ok()?
            .to_string(),
    };
    let start = node.start_position();
    let end = node.end_position();
    Some(Symbol {
        name,
        kind,
        file_id,
        path: path.clone(),
        line: start.row + 1,
        column: start.column + 1,
        end_line: end.row + 1,
    })
}

fn impl_name(node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let mut parts = Vec::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "scoped_type_identifier" | "generic_type" => {
                parts.push(child.utf8_text(source).ok()?.to_string());
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        Some("impl".to_string())
    } else {
        Some(parts.join(" for "))
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::{SymbolKind, extract_rust_symbols};

    #[test]
    fn extracts_rust_symbols() {
        let text = r#"
struct User;
enum Mode { A }
trait Run { fn run(&self); }
impl User { fn new() -> Self { User } }
fn main() {}
const X: u8 = 1;
type Id = u64;
mod inner {}
"#;
        let symbols = extract_rust_symbols(0, &Utf8PathBuf::from("main.rs"), text).unwrap();

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == SymbolKind::Struct)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "new" && s.kind == SymbolKind::Method)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "main" && s.kind == SymbolKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "inner" && s.kind == SymbolKind::Module)
        );
    }
}
