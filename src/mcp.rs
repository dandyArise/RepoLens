use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Result, anyhow};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::EditOpArg;
use crate::index::ProjectIndex;
use crate::{deps, edit, read, search, snapshot, symbols, watcher};

#[derive(Debug, Deserialize)]
struct Request {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

pub fn serve(root: &Path) -> Result<()> {
    let index = snapshot::load_or_build(root)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(&index, &line);
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_line(index: &ProjectIndex, line: &str) -> Response {
    let request: Result<Request, _> = serde_json::from_str(line);
    match request {
        Ok(request) => {
            let id = request.id.clone();
            match handle_request(index, request) {
                Ok(result) => Response {
                    jsonrpc: "2.0",
                    id,
                    result: Some(result),
                    error: None,
                },
                Err(error) => Response {
                    jsonrpc: "2.0",
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: -32000,
                        message: error.to_string(),
                    }),
                },
            }
        }
        Err(error) => Response {
            jsonrpc: "2.0",
            id: None,
            result: None,
            error: Some(RpcError {
                code: -32700,
                message: error.to_string(),
            }),
        },
    }
}

fn handle_request(index: &ProjectIndex, request: Request) -> Result<Value> {
    let _jsonrpc = request.jsonrpc.as_deref().unwrap_or("2.0");
    match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "repolens", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}}
        })),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call_tool(index, request.params.unwrap_or(Value::Null)),
        method => Err(anyhow!("unknown method: {method}")),
    }
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "repolens_status",
            "Return index status.",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "repolens_tree",
            "List indexed files.",
            json!({"type": "object", "properties": {"limit": {"type": "integer"}}}),
        ),
        tool(
            "repolens_search",
            "Search indexed content.",
            json!({"type": "object", "required": ["query"], "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}}),
        ),
        tool(
            "repolens_word",
            "Find files containing an identifier word.",
            json!({"type": "object", "required": ["word"], "properties": {"word": {"type": "string"}, "limit": {"type": "integer"}}}),
        ),
        tool(
            "repolens_read",
            "Read a file with optional line range.",
            json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}, "lines": {"type": "string"}, "max_bytes": {"type": "integer"}, "hash": {"type": "string"}}}),
        ),
        tool(
            "repolens_bundle",
            "Run multiple read-only RepoLens tools in one call.",
            json!({"type": "object", "required": ["ops"], "properties": {"ops": {"type": "array", "items": {"type": "object", "required": ["tool"], "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}}}}}}),
        ),
        tool(
            "repolens_outline",
            "Return symbols in one indexed file.",
            json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}}),
        ),
        tool(
            "repolens_symbol",
            "Find indexed symbols by name.",
            json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "limit": {"type": "integer"}}}),
        ),
        tool(
            "repolens_deps",
            "Return imports for one indexed file.",
            json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}}),
        ),
        tool(
            "repolens_rdeps",
            "Return files that import one indexed file.",
            json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}}),
        ),
        tool(
            "repolens_edit",
            "Apply a guarded line edit. Requires current file hash.",
            json!({"type": "object", "required": ["path", "op", "start", "hash"], "properties": {"path": {"type": "string"}, "op": {"type": "string", "enum": ["replace", "insert", "delete"]}, "start": {"type": "integer"}, "end": {"type": "integer"}, "content": {"type": "string"}, "hash": {"type": "string"}}}),
        ),
        tool(
            "repolens_changes",
            "Return latest watcher sequence and changed paths.",
            json!({"type": "object", "properties": {}}),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn call_tool(index: &ProjectIndex, params: Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    let content = call_tool_raw(index, name, args)?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&content)?
        }]
    }))
}

fn call_tool_raw(index: &ProjectIndex, name: &str, args: Value) -> Result<Value> {
    let value = match name {
        "repolens_status" => json!({
            "root": index.root,
            "files": index.files.len(),
            "words": index.words.len(),
            "trigrams": index.trigrams.len(),
            "symbols": index.symbols.len(),
            "symbol_names": index.symbols_by_name.len(),
            "deps_files": index.deps.len(),
            "deps_forward": index.deps_forward.len(),
            "deps_reverse": index.deps_reverse.len()
        }),
        "repolens_tree" => {
            let limit = get_usize(&args, "limit").unwrap_or(200);
            json!(index.files.iter().take(limit).map(|file| {
                json!({"path": file.path, "lines": file.lines, "bytes": file.bytes, "hash": file.hash})
            }).collect::<Vec<_>>())
        }
        "repolens_search" => {
            let query = get_str(&args, "query")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(search::search_hits(index, query, limit)?)
        }
        "repolens_word" => {
            let word = get_str(&args, "word")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(search::word_paths(index, word, limit))
        }
        "repolens_read" => {
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            let lines = args.get("lines").and_then(Value::as_str);
            let max_bytes = get_usize(&args, "max_bytes");
            let hash = args.get("hash").and_then(Value::as_str);
            json!(read::read_text(
                index.root.as_std_path(),
                &path,
                lines,
                max_bytes,
                hash
            )?)
        }
        "repolens_outline" => {
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(symbols::outline(index, &path))
        }
        "repolens_symbol" => {
            let name = get_str(&args, "name")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(symbols::find(index, name, limit))
        }
        "repolens_deps" => {
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(deps::deps_for_file(index, &path))
        }
        "repolens_rdeps" => {
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(deps::reverse_deps_for_file(index, &path))
        }
        "repolens_edit" => {
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            let op = parse_edit_op(get_str(&args, "op")?)?;
            let start = get_required_usize(&args, "start")?;
            let end = get_usize(&args, "end");
            let content = args.get("content").and_then(Value::as_str);
            let hash = get_str(&args, "hash")?;
            json!(edit::apply(
                index.root.as_std_path(),
                &path,
                op,
                start,
                end,
                content,
                hash
            )?)
        }
        "repolens_changes" => json!(watcher::read_state(&index.root)?),
        "repolens_bundle" => {
            let ops = args
                .get("ops")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing 'ops'"))?;
            let mut results = Vec::new();
            for op in ops.iter().take(10) {
                let tool = get_str(op, "tool")?;
                if tool == "repolens_bundle" {
                    return Err(anyhow!("nested bundle is not allowed"));
                }
                let arguments = op.get("arguments").cloned().unwrap_or(Value::Null);
                results.push(json!({
                    "tool": tool,
                    "result": call_tool_raw(index, tool, arguments)?
                }));
            }
            json!(results)
        }
        other => return Err(anyhow!("unknown tool: {other}")),
    };
    Ok(value)
}

fn get_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing '{key}'"))
}

fn get_usize(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn get_required_usize(value: &Value, key: &str) -> Result<usize> {
    get_usize(value, key).ok_or_else(|| anyhow!("missing '{key}'"))
}

fn parse_edit_op(raw: &str) -> Result<EditOpArg> {
    match raw {
        "replace" => Ok(EditOpArg::Replace),
        "insert" => Ok(EditOpArg::Insert),
        "delete" => Ok(EditOpArg::Delete),
        _ => Err(anyhow!("invalid edit op")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::handle_line;
    use crate::index::ProjectIndex;

    #[test]
    fn handles_tools_list() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let index = ProjectIndex::build(temp.path()).unwrap();

        let response = handle_line(&index, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);

        assert!(response.error.is_none());
        assert!(response.result.unwrap()["tools"].is_array());
    }

    #[test]
    fn handles_bundle() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let index = ProjectIndex::build(temp.path()).unwrap();

        let response = handle_line(
            &index,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"repolens_bundle","arguments":{"ops":[{"tool":"repolens_status","arguments":{}},{"tool":"repolens_search","arguments":{"query":"main","limit":1}}]}}}"#,
        );

        assert!(response.error.is_none());
        let text = response.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("repolens_status"));
        assert!(text.contains("repolens_search"));
    }
}
