use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Result, anyhow};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::index::ProjectIndex;
use crate::{read, search, snapshot};

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

    let content = match name {
        "repolens_status" => json!({
            "root": index.root,
            "files": index.files.len(),
            "words": index.words.len(),
            "trigrams": index.trigrams.len()
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
        other => return Err(anyhow!("unknown tool: {other}")),
    };

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&content)?
        }]
    }))
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
}
