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
    JavaImport,
    CInclude,
    CSharpUsing,
    RubyRequire,
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

            import.resolved_path = None;
            import.resolved_file_id = None;
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

pub fn extract_java_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    Ok(line_deps(
        file_id,
        path,
        text,
        ImportKind::JavaImport,
        |line| {
            Some(
                line.trim()
                    .strip_prefix("import ")?
                    .trim_end_matches(';')
                    .trim()
                    .to_string(),
            )
        },
    ))
}

pub fn extract_c_like_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    Ok(line_deps(
        file_id,
        path,
        text,
        ImportKind::CInclude,
        |line| {
            let line = line.trim();
            let rest = line.strip_prefix("#include")?.trim();
            Some(
                rest.trim_matches('<')
                    .trim_matches('>')
                    .trim_matches('"')
                    .to_string(),
            )
        },
    ))
}

pub fn extract_c_sharp_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    Ok(line_deps(
        file_id,
        path,
        text,
        ImportKind::CSharpUsing,
        |line| {
            Some(
                line.trim()
                    .strip_prefix("using ")?
                    .trim_end_matches(';')
                    .trim()
                    .to_string(),
            )
        },
    ))
}

pub fn extract_ruby_deps(file_id: FileId, path: &Utf8PathBuf, text: &str) -> Result<FileDeps> {
    Ok(line_deps(
        file_id,
        path,
        text,
        ImportKind::RubyRequire,
        |line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("require_relative ")
                .or_else(|| line.strip_prefix("require "))?;
            Some(clean_string(rest.trim()))
        },
    ))
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
        "import_statement" | "export_statement" => {
            if let Some(module) = node
                .child_by_field_name("source")
                .and_then(|node| literal_module(node, source))
            {
                imports.push(import(node, module, ImportKind::EsImport));
            }
        }
        "call_expression"
            if matches!(
                call_name(node, source).as_deref(),
                Some("require" | "import")
            ) =>
        {
            if let Some(arguments) = node.child_by_field_name("arguments")
                && let Some(argument) = arguments
                    .named_children(&mut arguments.walk())
                    .find(|node| node.kind() != "comment")
                && let Some(module) = literal_module(argument, source)
            {
                let kind = if call_name(node, source).as_deref() == Some("require") {
                    ImportKind::CommonJsRequire
                } else {
                    ImportKind::EsImport
                };
                imports.push(import(node, module, kind));
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

fn line_deps(
    file_id: FileId,
    path: &Utf8PathBuf,
    text: &str,
    kind: ImportKind,
    parse: fn(&str) -> Option<String>,
) -> FileDeps {
    let imports = text
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let module = parse(line)?;
            Some(ImportRef {
                module,
                line: idx + 1,
                kind: kind.clone(),
                resolved_path: None,
                resolved_file_id: None,
            })
        })
        .collect();
    FileDeps {
        file_id,
        path: path.clone(),
        imports,
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
    let requested = normalize_relative_path(&base.join(module))?;
    let prefer_javascript = matches!(from.extension(), Some("js" | "jsx" | "mjs" | "cjs"));
    for candidate in ts_js_candidates(&requested, prefer_javascript) {
        if let Some(id) = by_path.get(&candidate) {
            return Some((candidate, *id));
        }
    }
    None
}

fn ts_js_candidates(path: &Utf8Path, prefer_javascript: bool) -> Vec<Utf8PathBuf> {
    const TS_FIRST: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
    const JS_FIRST: &[&str] = &["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"];
    let extensions = if prefer_javascript {
        JS_FIRST
    } else {
        TS_FIRST
    };

    if path
        .extension()
        .is_some_and(|ext| extensions.contains(&ext))
    {
        return vec![path.to_path_buf()];
    }

    let mut candidates = vec![path.to_path_buf()];
    for ext in extensions {
        candidates.push(Utf8PathBuf::from(format!("{path}.{ext}")));
    }
    for ext in extensions {
        candidates.push(path.join(format!("index.{ext}")));
    }
    candidates
}

fn normalize_relative_path(path: &Utf8Path) -> Option<Utf8PathBuf> {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Utf8Component::Normal(part) => out.push(part),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn clean_rust_use(text: &str) -> String {
    text.trim()
        .trim_start_matches("use")
        .trim()
        .trim_end_matches(';')
        .to_string()
}

fn literal_module(node: Node, source: &[u8]) -> Option<String> {
    if !matches!(node.kind(), "string" | "template_string") || node.has_error() {
        return None;
    }
    if node
        .named_children(&mut node.walk())
        .any(|child| child.kind() == "template_substitution")
    {
        return None;
    }
    decode_js_literal(node.utf8_text(source).ok()?)
}

fn decode_js_literal(text: &str) -> Option<String> {
    let delimiter = text.chars().next()?;
    if !matches!(delimiter, '\'' | '"' | '`') || !text.ends_with(delimiter) || text.len() < 2 {
        return None;
    }
    let mut chars = text[1..text.len() - 1].chars().peekable();
    let mut decoded = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if ch == '\r' && delimiter == '`' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                decoded.push('\n');
            } else {
                decoded.push(ch);
            }
            continue;
        }
        match chars.next()? {
            '\n' | '\u{2028}' | '\u{2029}' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'b' => decoded.push('\u{8}'),
            'f' => decoded.push('\u{c}'),
            'v' => decoded.push('\u{b}'),
            '0' if !chars.peek().is_some_and(char::is_ascii_digit) => decoded.push('\0'),
            // Legacy octal escapes are deliberately not guessed as module names.
            '0'..='9' => return None,
            'x' => decoded.push(char::from_u32(js_hex_digits(&mut chars, 2)?)?),
            'u' => {
                let value = if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut value = 0u32;
                    let mut count = 0;
                    loop {
                        let ch = chars.next()?;
                        if ch == '}' {
                            break;
                        }
                        value = value.checked_mul(16)?.checked_add(ch.to_digit(16)?)?;
                        count += 1;
                    }
                    if count == 0 {
                        return None;
                    }
                    value
                } else {
                    js_hex_digits(&mut chars, 4)?
                };
                let value = if (0xd800..=0xdbff).contains(&value) {
                    if chars.next()? != '\\' || chars.next()? != 'u' {
                        return None;
                    }
                    let low = js_hex_digits(&mut chars, 4)?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return None;
                    }
                    0x10000 + ((value - 0xd800) << 10) + low - 0xdc00
                } else {
                    value
                };
                decoded.push(char::from_u32(value)?);
            }
            escaped => decoded.push(escaped),
        }
    }
    Some(decoded)
}

fn js_hex_digits(chars: &mut impl Iterator<Item = char>, count: usize) -> Option<u32> {
    let mut value = 0;
    for _ in 0..count {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    Some(value)
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
    #[test]
    fn javascript_module_literals_decode_escapes_and_static_templates() {
        let files = vec![file(0, "main.js"), file(1, "lib.js")];
        let source = concat!(
            r#"require('./lib\u002ejs'); require("./lib\x2ejs"); "#,
            r#"require(`./lib.js`); import(`./lib\u{2e}js`); "#,
            "require('./li\\\nb.js'); ",
            r#"import x from './lib\u002ejs'; export * from './lib\x2ejs';"#,
        );
        let mut deps = vec![super::extract_ts_like_deps(0, &"main.js".into(), source).unwrap()];
        super::resolve_relative_ts_js_imports(&mut deps, &files);
        assert_eq!(deps[0].imports.len(), 7);
        assert!(
            deps[0]
                .imports
                .iter()
                .all(|i| i.module == "./lib.js" && i.resolved_file_id == Some(1))
        );
        let (forward, reverse) = super::build_graph(&deps);
        assert_eq!(forward.get(&0), Some(&vec![1]));
        assert_eq!(reverse.get(&1), Some(&vec![0]));
    }

    #[test]
    fn javascript_module_literals_preserve_escaped_characters_without_evaluating_expressions() {
        let source = r#"require('./a\\b.js'); require('./it\'s.js'); require("./a\"b.js"); require(`./\${literal}.js`); require('./\uD83D\uDE00.js'); require(`./\u{1f600}.js`); require(`./${name}.js`); require('./base'+'.js'); require(`./base`+'.js');"#;
        let deps = super::extract_ts_like_deps(0, &"main.js".into(), source).unwrap();
        assert_eq!(
            deps.imports
                .iter()
                .map(|i| i.module.as_str())
                .collect::<Vec<_>>(),
            vec![
                "./a\\b.js",
                "./it's.js",
                "./a\"b.js",
                "./${literal}.js",
                "./😀.js",
                "./😀.js"
            ]
        );
    }

    #[test]
    fn reexports_and_literal_dynamic_imports_build_graph_edges() {
        let files = vec![file(0, "main.js"), file(1, "lib.js")];
        let mut deps = vec![
            super::extract_ts_like_deps(
                0,
                &"main.js".into(),
                "export {x} from './lib.js'; export * from './lib.js'; import('./lib.js');",
            )
            .unwrap(),
        ];
        super::resolve_relative_ts_js_imports(&mut deps, &files);
        assert_eq!(deps[0].imports.len(), 3);
        assert!(
            deps[0]
                .imports
                .iter()
                .all(|i| i.resolved_file_id == Some(1))
        );
        let (forward, reverse) = super::build_graph(&deps);
        assert_eq!(forward.get(&0), Some(&vec![1]));
        assert_eq!(reverse.get(&1), Some(&vec![0]));
    }

    #[test]
    fn computed_require_never_creates_a_false_literal_edge() {
        let deps = super::extract_ts_like_deps(
            0,
            &"main.js".into(),
            "require('./base'+'.js'); require(resolve('./other')); require('./literal.js');",
        )
        .unwrap();
        assert_eq!(
            deps.imports
                .iter()
                .map(|i| i.module.as_str())
                .collect::<Vec<_>>(),
            vec!["./literal.js"]
        );
    }
    use camino::Utf8PathBuf;

    use crate::index::FileEntry;

    use super::{
        FileDeps, ImportKind, ImportRef, build_graph, extract_c_like_deps, extract_c_sharp_deps,
        extract_go_deps, extract_java_deps, extract_php_deps, extract_python_deps,
        extract_ruby_deps, extract_rust_deps, extract_ts_like_deps, resolve_relative_ts_js_imports,
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
    fn extracts_java_csharp_c_and_ruby_deps() {
        let java = extract_java_deps(
            0,
            &Utf8PathBuf::from("Service.java"),
            "import java.util.List;\nclass Service {}\n",
        )
        .unwrap();
        let cs = extract_c_sharp_deps(
            1,
            &Utf8PathBuf::from("Service.cs"),
            "using System.Text;\nclass Service {}\n",
        )
        .unwrap();
        let c = extract_c_like_deps(
            2,
            &Utf8PathBuf::from("main.c"),
            "#include <stdio.h>\n#include \"app.h\"\n",
        )
        .unwrap();
        let rb = extract_ruby_deps(
            3,
            &Utf8PathBuf::from("app.rb"),
            "require 'json'\nrequire_relative './user'\n",
        )
        .unwrap();

        assert!(java.imports.iter().any(|i| i.module == "java.util.List"));
        assert!(cs.imports.iter().any(|i| i.module == "System.Text"));
        assert!(c.imports.iter().any(|i| i.module == "stdio.h"));
        assert!(c.imports.iter().any(|i| i.module == "app.h"));
        assert!(rb.imports.iter().any(|i| i.module == "json"));
        assert!(rb.imports.iter().any(|i| i.module == "./user"));
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
    fn resolves_dotted_module_names_and_reverse_dependencies() {
        let files = vec![
            file(0, "src/main.js"),
            file(1, "src/foo.bar.js"),
            file(2, "src/widgets.v2/index.ts"),
            file(3, "src/explicit.js"),
            file(4, "src/explicit.js.ts"),
        ];
        let mut deps = vec![extract_ts_like_deps(
            0,
            &Utf8PathBuf::from("src/main.js"),
            "const foo = require('./foo.bar');\nimport widget from './widgets.v2';\nimport explicit from './explicit.js';\n",
        ).unwrap()];

        resolve_relative_ts_js_imports(&mut deps, &files);

        for (import, target) in deps[0].imports.iter().zip(&files[1..4]) {
            assert_eq!(import.resolved_path.as_ref(), Some(&target.path));
            assert_eq!(import.resolved_file_id, Some(target.id));
        }
        let (forward, reverse) = build_graph(&deps);
        assert_eq!(forward.get(&0), Some(&vec![1, 2, 3]));
        for target in 1..=3 {
            assert_eq!(reverse.get(&target), Some(&vec![0]));
        }
    }

    #[test]
    fn rejects_imports_escaping_repository_and_clears_stale_graph_edges() {
        let files = vec![file(0, "main.js"), file(1, "shared.js")];
        let mut deps = vec![
            extract_ts_like_deps(
                0,
                &Utf8PathBuf::from("main.js"),
                "require('../shared'); require('./nested/../../shared');",
            )
            .unwrap(),
        ];
        for import in &mut deps[0].imports {
            import.resolved_path = Some(Utf8PathBuf::from("shared.js"));
            import.resolved_file_id = Some(1);
        }
        resolve_relative_ts_js_imports(&mut deps, &files);
        assert!(
            deps[0]
                .imports
                .iter()
                .all(|i| i.resolved_path.is_none() && i.resolved_file_id.is_none())
        );
        let (forward, reverse) = build_graph(&deps);
        assert!(forward.is_empty());
        assert!(reverse.is_empty());
    }

    #[test]
    fn resolves_parent_imports_within_repository_and_removes_deleted_targets() {
        let files = vec![file(0, "src/main.js"), file(1, "shared.js")];
        let mut deps = vec![
            extract_ts_like_deps(
                0,
                &Utf8PathBuf::from("src/main.js"),
                "require('../shared');",
            )
            .unwrap(),
        ];
        resolve_relative_ts_js_imports(&mut deps, &files);
        assert_eq!(deps[0].imports[0].resolved_file_id, Some(1));
        assert_eq!(build_graph(&deps).1.get(&1), Some(&vec![0]));
        resolve_relative_ts_js_imports(&mut deps, &files[..1]);
        assert_eq!(deps[0].imports[0].resolved_path, None);
        assert_eq!(deps[0].imports[0].resolved_file_id, None);
        assert!(build_graph(&deps).1.is_empty());
    }

    #[test]
    fn javascript_importers_prefer_javascript_and_typescript_keeps_its_priority() {
        let files = vec![
            file(1, "util.ts"),
            file(2, "util.js"),
            file(3, "dir/index.ts"),
            file(4, "dir/index.js"),
        ];
        for extension in ["js", "cjs", "mjs", "jsx", "ts", "tsx", "mts", "cts"] {
            let path = Utf8PathBuf::from(format!("main.{extension}"));
            let mut deps = vec![extract_ts_like_deps(0, &path,
                "require('./util'); require('./dir'); require('./util.ts'); require('./util.js');").unwrap()];
            resolve_relative_ts_js_imports(&mut deps, &files);
            let is_js = ["js", "cjs", "mjs", "jsx"].contains(&extension);
            assert_eq!(
                deps[0].imports[0].resolved_file_id,
                Some(if is_js { 2 } else { 1 }),
                "{extension}"
            );
            assert_eq!(
                deps[0].imports[1].resolved_file_id,
                Some(if is_js { 4 } else { 3 }),
                "{extension}"
            );
            assert_eq!(deps[0].imports[2].resolved_file_id, Some(1));
            assert_eq!(deps[0].imports[3].resolved_file_id, Some(2));
            assert_eq!(build_graph(&deps).1.get(&2), Some(&vec![0]));
        }
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
