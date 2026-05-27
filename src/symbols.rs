use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    Class,
    Struct,
    Enum,
    Interface,
    Trait,
    Impl,
    Type,
    Const,
    Module,
    Variable,
    Key,
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

pub fn extract_javascript_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .context("failed to load JavaScript tree-sitter grammar")?;
    extract_ts_like_symbols(file_id, path, text, parser)
}

pub fn extract_typescript_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
    tsx: bool,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    let language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    parser
        .set_language(&language.into())
        .context("failed to load TypeScript tree-sitter grammar")?;
    extract_ts_like_symbols(file_id, path, text, parser)
}

pub fn extract_python_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .context("failed to load Python tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Python file"))?;

    let mut symbols = Vec::new();
    walk_python(
        tree.root_node(),
        text.as_bytes(),
        file_id,
        path,
        false,
        &mut symbols,
    );
    Ok(symbols)
}

pub fn extract_go_symbols(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("failed to load Go tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse Go file"))?;

    let mut symbols = Vec::new();
    walk_go(
        tree.root_node(),
        text.as_bytes(),
        file_id,
        path,
        &mut symbols,
    );
    Ok(symbols)
}

pub fn extract_php_symbols(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .context("failed to load PHP tree-sitter grammar")?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse PHP file"))?;

    let mut symbols = Vec::new();
    walk_php(
        tree.root_node(),
        text.as_bytes(),
        file_id,
        path,
        &mut symbols,
    );
    Ok(symbols)
}

pub fn extract_java_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .context("failed to load Java tree-sitter grammar")?;
    extract_generic_symbols(file_id, path, text, parser, java_symbol_from_node)
}

pub fn extract_c_sharp_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .context("failed to load C# tree-sitter grammar")?;
    extract_generic_symbols(file_id, path, text, parser, c_sharp_symbol_from_node)
}

pub fn extract_c_symbols(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .context("failed to load C tree-sitter grammar")?;
    extract_generic_symbols(file_id, path, text, parser, c_cpp_symbol_from_node)
}

pub fn extract_cpp_symbols(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .context("failed to load C++ tree-sitter grammar")?;
    extract_generic_symbols(file_id, path, text, parser, c_cpp_symbol_from_node)
}

pub fn extract_ruby_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .context("failed to load Ruby tree-sitter grammar")?;
    extract_generic_symbols(file_id, path, text, parser, ruby_symbol_from_node)
}

pub fn extract_json_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let value: Value = serde_json::from_str(text).context("failed to parse JSON")?;
    let mut symbols = Vec::new();
    collect_json_keys(file_id, path, "", &value, &mut symbols);
    Ok(symbols)
}

pub fn extract_toml_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
) -> Result<Vec<Symbol>> {
    let value: toml::Value = toml::from_str(text).context("failed to parse TOML")?;
    let mut symbols = Vec::new();
    collect_toml_keys(file_id, path, "", &value, &mut symbols);
    Ok(symbols)
}

pub fn extract_yaml_symbols(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Vec<Symbol> {
    text.lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || !trimmed.contains(':') {
                return None;
            }
            let key = trimmed.split_once(':')?.0.trim();
            if key.is_empty() || key.starts_with('-') {
                return None;
            }
            Some(Symbol {
                name: key.to_string(),
                kind: SymbolKind::Key,
                file_id,
                path: path.clone(),
                line: idx + 1,
                column: line.len() - trimmed.len() + 1,
                end_line: idx + 1,
            })
        })
        .collect()
}

fn extract_ts_like_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
    mut parser: Parser,
) -> Result<Vec<Symbol>> {
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse JS/TS file"))?;

    let mut symbols = Vec::new();
    walk_ts_like(
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
    for symbol in find(index, name, limit) {
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
    if let Some(ids) = index.symbols_by_name.get(&needle) {
        return ids
            .iter()
            .take(limit)
            .filter_map(|id| index.symbols.get(*id as usize))
            .cloned()
            .collect();
    }

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

fn walk_ts_like(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_class: bool,
    symbols: &mut Vec<Symbol>,
) {
    if let Some(symbol) = ts_like_symbol_from_node(node, source, file_id, path, in_class) {
        symbols.push(symbol);
    }

    let next_in_class = in_class || node.kind() == "class_body";
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ts_like(child, source, file_id, path, next_in_class, symbols);
    }
}

fn walk_python(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_class: bool,
    symbols: &mut Vec<Symbol>,
) {
    if let Some(symbol) = python_symbol_from_node(node, source, file_id, path, in_class) {
        symbols.push(symbol);
    }

    let next_in_class = in_class || node.kind() == "class_definition";
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python(child, source, file_id, path, next_in_class, symbols);
    }
}

fn walk_go(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    symbols: &mut Vec<Symbol>,
) {
    if let Some(symbol) = go_symbol_from_node(node, source, file_id, path) {
        symbols.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go(child, source, file_id, path, symbols);
    }
}

fn walk_php(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    symbols: &mut Vec<Symbol>,
) {
    if let Some(symbol) = php_symbol_from_node(node, source, file_id, path) {
        symbols.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php(child, source, file_id, path, symbols);
    }
}

fn extract_generic_symbols(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
    mut parser: Parser,
    symbol_fn: fn(Node, &[u8], FileId, &Utf8PathBuf) -> Option<Symbol>,
) -> Result<Vec<Symbol>> {
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse file"))?;
    let mut symbols = Vec::new();
    walk_generic(
        tree.root_node(),
        text.as_bytes(),
        file_id,
        path,
        &mut symbols,
        symbol_fn,
    );
    Ok(symbols)
}

fn walk_generic(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    symbols: &mut Vec<Symbol>,
    symbol_fn: fn(Node, &[u8], FileId, &Utf8PathBuf) -> Option<Symbol>,
) {
    if let Some(symbol) = symbol_fn(node, source, file_id, path) {
        symbols.push(symbol);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_generic(child, source, file_id, path, symbols, symbol_fn);
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

fn ts_like_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_class: bool,
) -> Option<Symbol> {
    let (kind, name) = match node.kind() {
        "function_declaration" => (
            SymbolKind::Function,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "method_definition" | "method_signature" => (
            SymbolKind::Method,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "class_declaration" => (
            SymbolKind::Class,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "interface_declaration" => (
            SymbolKind::Interface,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "type_alias_declaration" => (
            SymbolKind::Type,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "lexical_declaration" | "variable_declaration" => variable_symbol(node, source, in_class)?,
        _ => return None,
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

fn variable_symbol(node: Node, source: &[u8], in_class: bool) -> Option<(SymbolKind, String)> {
    if in_class {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = child
                .child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string();
            let value_kind = child.child_by_field_name("value").map(|value| value.kind());
            let kind = match value_kind {
                Some("arrow_function") | Some("function_expression") => SymbolKind::Function,
                Some("class") | Some("class_declaration") => SymbolKind::Class,
                _ => SymbolKind::Variable,
            };
            return Some((kind, name));
        }
    }
    None
}

fn python_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    in_class: bool,
) -> Option<Symbol> {
    let (kind, name) = match node.kind() {
        "function_definition" => (
            if in_class {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "class_definition" => (
            SymbolKind::Class,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "assignment" if !in_class => (
            SymbolKind::Variable,
            node.child_by_field_name("left")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        _ => return None,
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

fn go_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let (kind, name) = match node.kind() {
        "function_declaration" => (
            SymbolKind::Function,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "method_declaration" => (
            SymbolKind::Method,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "type_spec" => (
            SymbolKind::Type,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "const_spec" => (
            SymbolKind::Const,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "var_spec" => (
            SymbolKind::Variable,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        _ => return None,
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

fn php_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let (kind, name) = match node.kind() {
        "function_definition" => (
            SymbolKind::Function,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .trim_start_matches('$')
                .to_string(),
        ),
        "method_declaration" => (
            SymbolKind::Method,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "class_declaration" => (
            SymbolKind::Class,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "interface_declaration" => (
            SymbolKind::Interface,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        "trait_declaration" => (
            SymbolKind::Trait,
            node.child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .to_string(),
        ),
        _ => return None,
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

fn java_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let kind = match node.kind() {
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "method_declaration" | "constructor_declaration" => SymbolKind::Method,
        "field_declaration" => SymbolKind::Variable,
        _ => return None,
    };
    named_symbol(node, source, file_id, path, kind)
}

fn c_sharp_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let kind = match node.kind() {
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "struct_declaration" => SymbolKind::Struct,
        "method_declaration" | "constructor_declaration" => SymbolKind::Method,
        "property_declaration" | "field_declaration" => SymbolKind::Variable,
        _ => return None,
    };
    named_symbol(node, source, file_id, path, kind)
}

fn c_cpp_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let kind = match node.kind() {
        "function_definition" => SymbolKind::Function,
        "struct_specifier" => SymbolKind::Struct,
        "class_specifier" => SymbolKind::Class,
        "enum_specifier" => SymbolKind::Enum,
        "type_definition" | "alias_declaration" => SymbolKind::Type,
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok().map(str::to_string))
        .or_else(|| first_identifier(node, source))?;
    Some(make_symbol(node, file_id, path, kind, name))
}

fn ruby_symbol_from_node(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
) -> Option<Symbol> {
    let kind = match node.kind() {
        "class" => SymbolKind::Class,
        "module" => SymbolKind::Module,
        "method" | "singleton_method" => SymbolKind::Function,
        "assignment" => SymbolKind::Variable,
        _ => return None,
    };
    named_symbol(node, source, file_id, path, kind.clone()).or_else(|| {
        first_identifier(node, source).map(|name| make_symbol(node, file_id, path, kind, name))
    })
}

fn named_symbol(
    node: Node,
    source: &[u8],
    file_id: FileId,
    path: &Utf8PathBuf,
    kind: SymbolKind,
) -> Option<Symbol> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source)
        .ok()?
        .trim_start_matches('$')
        .to_string();
    Some(make_symbol(node, file_id, path, kind, name))
}

fn make_symbol(
    node: Node,
    file_id: FileId,
    path: &Utf8PathBuf,
    kind: SymbolKind,
    name: String,
) -> Symbol {
    let start = node.start_position();
    let end = node.end_position();
    Symbol {
        name,
        kind,
        file_id,
        path: path.clone(),
        line: start.row + 1,
        column: start.column + 1,
        end_line: end.row + 1,
    }
}

fn first_identifier(node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "field_identifier" | "constant"
        ) {
            return child.utf8_text(source).ok().map(str::to_string);
        }
        if let Some(found) = first_identifier(child, source) {
            return Some(found);
        }
    }
    None
}

fn collect_json_keys(
    file_id: FileId,
    path: &Utf8PathBuf,
    prefix: &str,
    value: &Value,
    symbols: &mut Vec<Symbol>,
) {
    if let Value::Object(map) = value {
        for (key, value) in map {
            let name = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            symbols.push(config_symbol(file_id, path, name.clone()));
            collect_json_keys(file_id, path, &name, value, symbols);
        }
    }
}

fn collect_toml_keys(
    file_id: FileId,
    path: &Utf8PathBuf,
    prefix: &str,
    value: &toml::Value,
    symbols: &mut Vec<Symbol>,
) {
    if let toml::Value::Table(map) = value {
        for (key, value) in map {
            let name = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            symbols.push(config_symbol(file_id, path, name.clone()));
            collect_toml_keys(file_id, path, &name, value, symbols);
        }
    }
}

fn config_symbol(file_id: FileId, path: &Utf8PathBuf, name: String) -> Symbol {
    Symbol {
        name,
        kind: SymbolKind::Key,
        file_id,
        path: path.clone(),
        line: 1,
        column: 1,
        end_line: 1,
    }
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

    use super::{
        SymbolKind, extract_c_sharp_symbols, extract_c_symbols, extract_cpp_symbols,
        extract_go_symbols, extract_java_symbols, extract_json_symbols, extract_php_symbols,
        extract_python_symbols, extract_ruby_symbols, extract_rust_symbols, extract_toml_symbols,
        extract_typescript_symbols, extract_yaml_symbols,
    };

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

    #[test]
    fn extracts_typescript_symbols() {
        let text = r#"
interface User { id: string }
type Id = string;
class Service { run() {} }
function makeUser() {}
const helper = () => {};
let value = 1;
"#;
        let symbols =
            extract_typescript_symbols(0, &Utf8PathBuf::from("main.ts"), text, false).unwrap();

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == SymbolKind::Interface)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Id" && s.kind == SymbolKind::Type)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Service" && s.kind == SymbolKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "run" && s.kind == SymbolKind::Method)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "makeUser" && s.kind == SymbolKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "helper" && s.kind == SymbolKind::Function)
        );
    }

    #[test]
    fn extracts_python_symbols() {
        let text = r#"
VALUE = 1

class Service:
    def run(self):
        pass

def make_user():
    return Service()
"#;
        let symbols = extract_python_symbols(0, &Utf8PathBuf::from("main.py"), text).unwrap();

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "VALUE" && s.kind == SymbolKind::Variable)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Service" && s.kind == SymbolKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "run" && s.kind == SymbolKind::Method)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "make_user" && s.kind == SymbolKind::Function)
        );
    }

    #[test]
    fn extracts_go_symbols() {
        let text = r#"
package main

type User struct {}
const Version = "1"
var Count = 1
func NewUser() User { return User{} }
func (u User) Run() {}
"#;
        let symbols = extract_go_symbols(0, &Utf8PathBuf::from("main.go"), text).unwrap();

        assert!(symbols.iter().any(|s| s.name == "User"));
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "NewUser" && s.kind == SymbolKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Run" && s.kind == SymbolKind::Method)
        );
    }

    #[test]
    fn extracts_php_symbols() {
        let text = r#"
<?php
class Service { public function run() {} }
interface Contract {}
function make_user() {}
"#;
        let symbols = extract_php_symbols(0, &Utf8PathBuf::from("main.php"), text).unwrap();

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Service" && s.kind == SymbolKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Contract" && s.kind == SymbolKind::Interface)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "make_user" && s.kind == SymbolKind::Function)
        );
    }

    #[test]
    fn extracts_java_symbols() {
        let text = "package app; import java.util.List; class Service { void run() {} } interface Contract {}";
        let symbols = extract_java_symbols(0, &Utf8PathBuf::from("Service.java"), text).unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Service" && s.kind == SymbolKind::Class)
        );
        assert!(symbols.iter().any(|s| s.name == "run"));
    }

    #[test]
    fn extracts_c_sharp_symbols() {
        let text = "using System; class Service { void Run() {} } interface IRun {}";
        let symbols = extract_c_sharp_symbols(0, &Utf8PathBuf::from("Service.cs"), text).unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Service" && s.kind == SymbolKind::Class)
        );
        assert!(symbols.iter().any(|s| s.name == "Run"));
    }

    #[test]
    fn extracts_c_and_cpp_symbols() {
        let c_symbols = extract_c_symbols(
            0,
            &Utf8PathBuf::from("main.c"),
            "#include <stdio.h>\nstruct User { int id; };\nint run() { return 1; }\n",
        )
        .unwrap();
        let cpp_symbols = extract_cpp_symbols(
            0,
            &Utf8PathBuf::from("main.cpp"),
            "#include <vector>\nclass Service { void run() {} };\nint main() { return 0; }\n",
        )
        .unwrap();
        assert!(c_symbols.iter().any(|s| s.name == "run"));
        assert!(cpp_symbols.iter().any(|s| s.name == "Service"));
    }

    #[test]
    fn extracts_ruby_symbols() {
        let text = "require 'json'\nmodule App\nclass Service\n def run\n end\nend\nend\n";
        let symbols = extract_ruby_symbols(0, &Utf8PathBuf::from("app.rb"), text).unwrap();
        assert!(symbols.iter().any(|s| s.name == "App"));
        assert!(symbols.iter().any(|s| s.name == "Service"));
        assert!(symbols.iter().any(|s| s.name == "run"));
    }

    #[test]
    fn extracts_config_symbols() {
        let json = extract_json_symbols(
            0,
            &Utf8PathBuf::from("package.json"),
            r#"{"scripts":{"test":"x"}}"#,
        )
        .unwrap();
        let toml =
            extract_toml_symbols(0, &Utf8PathBuf::from("Cargo.toml"), "[package]\nname='x'\n")
                .unwrap();
        let yaml = extract_yaml_symbols(
            0,
            &Utf8PathBuf::from("config.yml"),
            "name: app\nnested:\n  value: 1\n",
        );
        assert!(json.iter().any(|s| s.name == "scripts.test"));
        assert!(toml.iter().any(|s| s.name == "package.name"));
        assert!(yaml.iter().any(|s| s.name == "name"));
    }
}
