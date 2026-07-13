use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use toml_edit::{DocumentMut, Item};

use crate::cli::EditOpArg;
use crate::index::ProjectIndex;
use crate::pathing::canonical_utf8;
use crate::{deps, edit, read, search, snapshot, symbols, watcher};

const MAX_CACHED_ROOTS: usize = 16;
const ROOTS_ENV: &str = "REPOLENS_MCP_ROOTS";

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
    #[serde(skip_serializing_if = "Option::is_none")]
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

pub fn serve(root: Option<&Path>) -> Result<()> {
    let mut state = match root {
        Some(root) => ServerState::new(root)?,
        None => ServerState::without_root()?,
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if is_notification(&line) {
            continue;
        }
        let response = handle_line(&mut state, &line);
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }

    Ok(())
}

struct ServerState {
    active_root: Option<Utf8PathBuf>,
    allowed_roots: HashMap<String, Utf8PathBuf>,
    indexes: HashMap<String, ProjectIndex>,
    cache_order: VecDeque<String>,
}

impl ServerState {
    fn new(root: &Path) -> Result<Self> {
        let allowed_roots = allowed_workspace_roots(Some(root))?;
        Self::with_allowed_roots(root, allowed_roots.into_values())
    }

    fn without_root() -> Result<Self> {
        Ok(Self {
            active_root: None,
            allowed_roots: allowed_workspace_roots(None)?,
            indexes: HashMap::new(),
            cache_order: VecDeque::new(),
        })
    }

    fn with_allowed_roots(
        root: &Path,
        allowed_roots: impl IntoIterator<Item = Utf8PathBuf>,
    ) -> Result<Self> {
        let index = snapshot::load_or_build(root)?;
        let active_root = index.root.clone();
        let mut allowed = HashMap::new();
        add_allowed_root(&mut allowed, active_root.clone());
        for allowed_root in allowed_roots {
            add_allowed_root(&mut allowed, allowed_root);
        }

        let mut indexes = HashMap::new();
        let key = root_key(&active_root);
        indexes.insert(key.clone(), index);
        let mut cache_order = VecDeque::new();
        cache_order.push_back(key);

        Ok(Self {
            active_root: Some(active_root),
            allowed_roots: allowed,
            indexes,
            cache_order,
        })
    }

    fn cached_roots(&self) -> Vec<Utf8PathBuf> {
        let mut roots = self
            .indexes
            .values()
            .map(|index| index.root.clone())
            .collect::<Vec<_>>();
        roots.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        roots
    }

    fn allowed_roots(&self) -> Vec<Utf8PathBuf> {
        let mut roots = self.allowed_roots.values().cloned().collect::<Vec<_>>();
        roots.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        roots
    }

    fn switch_root(&mut self, root: &Path) -> Result<()> {
        let root = self.allowed_root(root)?;
        let key = root_key(&root);
        if !self.indexes.contains_key(&key) {
            let index = snapshot::load_or_build(root.as_std_path())?;
            self.indexes.insert(key.clone(), index);
        }
        self.active_root = Some(root);
        self.touch_cache_key(key);
        self.evict_inactive_roots();
        Ok(())
    }

    fn index_for_args(&mut self, args: &Value) -> Result<&ProjectIndex> {
        if let Some(root) = workspace_root_arg(args) {
            self.switch_root(Path::new(root))?;
        }
        let active_root = self.active_root.as_ref().ok_or_else(|| {
            anyhow!(
                "workspace root is required; pass workspaceRoot or call repolens_switch_workspace"
            )
        })?;
        let key = root_key(active_root);
        self.indexes
            .get(&key)
            .ok_or_else(|| anyhow!("active workspace is not indexed: {active_root}"))
    }

    fn allowed_root(&self, root: &Path) -> Result<Utf8PathBuf> {
        reject_unsafe_workspace_root(root)?;
        let root = canonical_utf8(root)?;
        let key = root_key(&root);
        self.allowed_roots.get(&key).cloned().ok_or_else(|| {
            anyhow!(
                "workspace root is not allowed: {}. Run `repolens init . --target codex` from that project or set {ROOTS_ENV}.",
                root
            )
        })
    }

    fn touch_cache_key(&mut self, key: String) {
        self.cache_order.retain(|cached| cached != &key);
        self.cache_order.push_back(key);
    }

    fn evict_inactive_roots(&mut self) {
        while self.indexes.len() > MAX_CACHED_ROOTS {
            let Some(candidate) = self.cache_order.pop_front() else {
                break;
            };
            if self
                .active_root
                .as_ref()
                .is_some_and(|active_root| candidate == root_key(active_root))
            {
                self.cache_order.push_back(candidate);
                break;
            }
            self.indexes.remove(&candidate);
        }
    }
}

fn root_key(root: &Utf8Path) -> String {
    let key = root
        .as_str()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

fn allowed_workspace_roots(initial_root: Option<&Path>) -> Result<HashMap<String, Utf8PathBuf>> {
    let mut roots = HashMap::new();
    if let Some(initial_root) = initial_root {
        add_allowed_root(&mut roots, canonical_utf8(initial_root)?);
    }
    for root in env_allowed_roots().into_iter().chain(codex_config_roots()) {
        if let Ok(root) = canonical_utf8(&root) {
            add_allowed_root(&mut roots, root);
        }
    }
    Ok(roots)
}

fn add_allowed_root(roots: &mut HashMap<String, Utf8PathBuf>, root: Utf8PathBuf) {
    roots.insert(root_key(&root), root);
}

fn env_allowed_roots() -> Vec<PathBuf> {
    std::env::var_os(ROOTS_ENV)
        .map(|raw| {
            raw.to_string_lossy()
                .split(';')
                .map(str::trim)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn codex_config_roots() -> Vec<PathBuf> {
    let path = home_dir().join(".codex").join("config.toml");
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(document) = raw.parse::<DocumentMut>() else {
        return Vec::new();
    };
    let Some(servers) = document.get("mcp_servers").and_then(Item::as_table) else {
        return Vec::new();
    };

    servers
        .iter()
        .filter_map(|(_, item)| codex_entry_root(item))
        .collect()
}

fn codex_entry_root(item: &Item) -> Option<PathBuf> {
    let table = item.as_table()?;
    let command = table.get("command")?.as_str()?;
    let executable = command
        .replace('\\', "/")
        .rsplit('/')
        .next()?
        .to_ascii_lowercase();
    if executable != "repolens" && executable != "repolens.exe" {
        return None;
    }

    let args = table.get("args")?.as_array()?;
    if args.get(0)?.as_str()? != "mcp" {
        return None;
    }
    Some(PathBuf::from(args.get(1)?.as_str()?))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn reject_unsafe_workspace_root(root: &Path) -> Result<()> {
    if root.as_os_str().is_empty() {
        bail!("workspace root is empty");
    }
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!(
            "unsafe workspace root contains parent traversal: {}",
            root.display()
        );
    }
    Ok(())
}

fn workspace_root_arg(args: &Value) -> Option<&str> {
    args.get("workspaceRoot")
        .or_else(|| args.get("root"))
        .and_then(Value::as_str)
}

fn handle_line(state: &mut ServerState, line: &str) -> Response {
    let request: Result<Request, _> = serde_json::from_str(line);
    match request {
        Ok(request) => {
            let id = request.id.clone();
            match handle_request(state, request) {
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

fn handle_request(state: &mut ServerState, request: Request) -> Result<Value> {
    let _jsonrpc = request.jsonrpc.as_deref().unwrap_or("2.0");
    match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "repolens", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}}
        })),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call_tool(state, request.params.unwrap_or(Value::Null)),
        method => Err(anyhow!("unknown method: {method}")),
    }
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "repolens_status",
            "Return index status.",
            schema(json!({})),
        ),
        tool(
            "repolens_switch_workspace",
            "Switch the active workspace root for subsequent RepoLens calls in this MCP process.",
            schema(json!({})).any_required([vec!["workspaceRoot"], vec!["root"]]),
        ),
        tool(
            "repolens_snapshot",
            "Return snapshot path and metadata.",
            schema(json!({})),
        ),
        tool(
            "repolens_tree",
            "List indexed files.",
            schema(json!({"limit": {"type": "integer"}})),
        ),
        tool(
            "repolens_search",
            "Search indexed content.",
            schema(json!({"query": {"type": "string"}, "limit": {"type": "integer"}})).required(["query"]),
        ),
        tool(
            "repolens_word",
            "Find files containing an identifier word.",
            schema(json!({"word": {"type": "string"}, "limit": {"type": "integer"}})).required(["word"]),
        ),
        tool(
            "repolens_read",
            "Read a file with optional line range.",
            schema(json!({"path": {"type": "string"}, "lines": {"type": "string"}, "max_bytes": {"type": "integer"}, "hash": {"type": "string"}})).required(["path"]),
        ),
        tool(
            "repolens_bundle",
            "Run multiple read-only RepoLens tools in one call.",
            schema(json!({"ops": {"type": "array", "items": {"type": "object", "required": ["tool"], "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}}}}})).required(["ops"]),
        ),
        tool(
            "repolens_outline",
            "Return symbols in one indexed file.",
            schema(json!({"path": {"type": "string"}})).required(["path"]),
        ),
        tool(
            "repolens_symbol",
            "Find indexed symbols by name.",
            schema(json!({"name": {"type": "string"}, "limit": {"type": "integer"}})).required(["name"]),
        ),
        tool(
            "repolens_deps",
            "Return imports for one indexed file.",
            schema(json!({"path": {"type": "string"}})).required(["path"]),
        ),
        tool(
            "repolens_rdeps",
            "Return files that import one indexed file.",
            schema(json!({"path": {"type": "string"}})).required(["path"]),
        ),
        tool(
            "repolens_edit",
            "Apply a guarded line edit. Requires current file hash.",
            schema(json!({"path": {"type": "string"}, "op": {"type": "string", "enum": ["replace", "insert", "delete"]}, "start": {"type": "integer"}, "end": {"type": "integer"}, "content": {"type": "string"}, "hash": {"type": "string"}})).required(["path", "op", "start", "hash"]),
        ),
        tool(
            "repolens_changes",
            "Return latest watcher sequence and changed paths.",
            schema(json!({})),
        ),
    ]
}

trait SchemaRequired {
    fn required(self, required: impl IntoIterator<Item = &'static str>) -> Value;
    fn any_required(self, required_groups: impl IntoIterator<Item = Vec<&'static str>>) -> Value;
}

impl SchemaRequired for Value {
    fn required(mut self, required: impl IntoIterator<Item = &'static str>) -> Value {
        self["required"] = json!(required.into_iter().collect::<Vec<_>>());
        self
    }

    fn any_required(
        mut self,
        required_groups: impl IntoIterator<Item = Vec<&'static str>>,
    ) -> Value {
        self["anyOf"] = json!(
            required_groups
                .into_iter()
                .map(|group| json!({"required": group}))
                .collect::<Vec<_>>()
        );
        self
    }
}

fn schema(properties: Value) -> Value {
    let mut properties = properties.as_object().cloned().unwrap_or_default();
    properties.insert(
        "workspaceRoot".to_owned(),
        json!({
            "type": "string",
            "description": "Optional repository root to use for this call. Also switches the MCP process active root."
        }),
    );
    properties.insert(
        "root".to_owned(),
        json!({
            "type": "string",
            "description": "Alias for workspaceRoot."
        }),
    );
    json!({"type": "object", "properties": properties})
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn call_tool(state: &mut ServerState, params: Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    match call_tool_raw(state, name, args) {
        Ok(content) => Ok(json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&content)?
            }]
        })),
        Err(error) => Ok(json!({
            "content": [{
                "type": "text",
                "text": error.to_string()
            }],
            "isError": true
        })),
    }
}

fn is_notification(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|message| message.get("method").is_some() && message.get("id").is_none())
}

fn call_tool_raw(state: &mut ServerState, name: &str, args: Value) -> Result<Value> {
    let previous_active = state.active_root.clone();
    let result = call_tool_raw_inner(state, name, args);
    if result.is_err() {
        state.active_root = previous_active;
    }
    result
}

fn call_tool_raw_inner(state: &mut ServerState, name: &str, args: Value) -> Result<Value> {
    let value = match name {
        "repolens_switch_workspace" => {
            let root =
                workspace_root_arg(&args).ok_or_else(|| anyhow!("missing 'workspaceRoot'"))?;
            state.switch_root(Path::new(root))?;
            json!({
                "root": state.active_root,
                "active_root": state.active_root,
                "cached_roots": state.cached_roots(),
                "allowed_roots": state.allowed_roots()
            })
        }
        "repolens_status" => {
            let (
                root,
                files,
                words,
                trigrams,
                symbols,
                symbol_names,
                deps_files,
                deps_forward,
                deps_reverse,
            ) = {
                let index = state.index_for_args(&args)?;
                (
                    index.root.clone(),
                    index.files.len(),
                    index.words.len(),
                    index.trigrams.len(),
                    index.symbols.len(),
                    index.symbols_by_name.len(),
                    index.deps.len(),
                    index.deps_forward.len(),
                    index.deps_reverse.len(),
                )
            };
            json!({
                "root": root,
                "active_root": state.active_root,
                "cached_roots": state.cached_roots(),
                "allowed_roots": state.allowed_roots(),
                "files": files,
                "words": words,
                "trigrams": trigrams,
                "symbols": symbols,
                "symbol_names": symbol_names,
                "deps_files": deps_files,
                "deps_forward": deps_forward,
                "deps_reverse": deps_reverse
            })
        }
        "repolens_snapshot" => {
            let index = state.index_for_args(&args)?;
            json!(snapshot::info(index))
        }
        "repolens_tree" => {
            let index = state.index_for_args(&args)?;
            let limit = get_usize(&args, "limit").unwrap_or(200);
            json!(index.files.iter().take(limit).map(|file| {
                json!({"path": file.path, "lines": file.lines, "bytes": file.bytes, "hash": file.hash})
            }).collect::<Vec<_>>())
        }
        "repolens_search" => {
            let index = state.index_for_args(&args)?;
            let query = get_str(&args, "query")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(search::search_hits(index, query, limit)?)
        }
        "repolens_word" => {
            let index = state.index_for_args(&args)?;
            let word = get_str(&args, "word")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(search::word_paths(index, word, limit))
        }
        "repolens_read" => {
            let index = state.index_for_args(&args)?;
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
            let index = state.index_for_args(&args)?;
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(symbols::outline(index, &path))
        }
        "repolens_symbol" => {
            let index = state.index_for_args(&args)?;
            let name = get_str(&args, "name")?;
            let limit = get_usize(&args, "limit").unwrap_or(20);
            json!(symbols::find(index, name, limit))
        }
        "repolens_deps" => {
            let index = state.index_for_args(&args)?;
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(deps::deps_for_file(index, &path))
        }
        "repolens_rdeps" => {
            let index = state.index_for_args(&args)?;
            let path = Utf8PathBuf::from(get_str(&args, "path")?);
            json!(deps::reverse_deps_for_file(index, &path))
        }
        "repolens_edit" => {
            let index = state.index_for_args(&args)?;
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
        "repolens_changes" => {
            let index = state.index_for_args(&args)?;
            json!(watcher::read_state(&index.root)?)
        }
        "repolens_bundle" => {
            let workspace_root = workspace_root_arg(&args).map(str::to_owned);
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
                let mut arguments = op.get("arguments").cloned().unwrap_or(Value::Null);
                if let Some(root) = &workspace_root
                    && workspace_root_arg(&arguments).is_none()
                {
                    ensure_object(&mut arguments)?;
                    arguments["workspaceRoot"] = json!(root);
                }
                results.push(json!({
                    "tool": tool,
                    "result": call_tool_raw(state, tool, arguments)?
                }));
            }
            json!(results)
        }
        other => return Err(anyhow!("unknown tool: {other}")),
    };
    Ok(value)
}

fn ensure_object(value: &mut Value) -> Result<()> {
    if value.is_null() {
        *value = json!({});
    }
    if value.is_object() {
        Ok(())
    } else {
        Err(anyhow!("tool arguments must be an object"))
    }
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

    use serde_json::json;

    use super::{MAX_CACHED_ROOTS, ServerState, handle_line, is_notification, tools};

    fn canonical(path: &std::path::Path) -> camino::Utf8PathBuf {
        camino::Utf8PathBuf::from_path_buf(dunce::canonicalize(path).unwrap()).unwrap()
    }

    fn tool_text(response: super::Response) -> String {
        response.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn tool_json(response: super::Response) -> serde_json::Value {
        serde_json::from_str(&tool_text(response)).unwrap()
    }

    #[test]
    fn handles_tools_list() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut state = ServerState::new(temp.path()).unwrap();

        let response = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );

        assert!(response.error.is_none());
        assert!(response.result.unwrap()["tools"].is_array());
    }

    #[test]
    fn starts_without_a_workspace_root() {
        let mut state = ServerState::without_root().unwrap();
        let response = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );

        assert!(response.error.is_none());
        assert!(response.result.unwrap()["tools"].is_array());
        assert!(state.active_root.is_none());
    }

    #[test]
    fn handles_bundle() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut state = ServerState::new(temp.path()).unwrap();

        let response = handle_line(
            &mut state,
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

    #[test]
    fn omits_id_for_parse_errors() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = ServerState::new(temp.path()).unwrap();

        let response = handle_line(&mut state, "{");
        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(serialized["error"]["code"], -32700);
        assert!(serialized.get("id").is_none());
    }

    #[test]
    fn recognizes_notifications_without_an_id() {
        assert!(is_notification(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        ));
        assert!(!is_notification(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#
        ));
    }

    #[test]
    fn returns_tool_errors_as_mcp_results() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = ServerState::new(temp.path()).unwrap();

        let response = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"repolens_search","arguments":{}}}"#,
        );

        assert!(response.error.is_none());
        assert_eq!(response.id, Some(json!(12)));
        assert_eq!(response.result.unwrap()["isError"], true);
    }

    #[test]
    fn switches_workspace_root_from_tool_arguments() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        fs::write(second.path().join("second.rs"), "fn beta() {}\n").unwrap();
        let mut state =
            ServerState::with_allowed_roots(first.path(), [canonical(second.path())]).unwrap();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_search",
                "arguments": {
                    "query": "beta",
                    "workspaceRoot": second.path()
                }
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        let text = tool_text(response);
        assert!(text.contains("second.rs"));

        let status = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"repolens_status","arguments":{}}}"#,
        );
        let status = tool_json(status);
        let second_root = dunce::canonicalize(second.path()).unwrap();
        assert_eq!(
            status["root"].as_str().unwrap().replace('\\', "/"),
            second_root.to_string_lossy().replace('\\', "/")
        );
    }

    #[test]
    fn switch_workspace_tool_accepts_root_alias() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        fs::write(second.path().join("second.rs"), "fn beta() {}\n").unwrap();
        let mut state =
            ServerState::with_allowed_roots(first.path(), [canonical(second.path())]).unwrap();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_switch_workspace",
                "arguments": {"root": second.path()}
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        let text = tool_text(response);
        assert!(!text.contains("second.rs"));
        assert!(text.contains("allowed_roots"));
        assert_eq!(state.active_root, Some(canonical(second.path())));
    }

    #[test]
    fn rejects_unregistered_workspace_and_keeps_active_root() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        fs::write(second.path().join("second.rs"), "fn beta() {}\n").unwrap();
        let mut state = ServerState::new(first.path()).unwrap();
        let first_root = state.active_root.clone();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_search",
                "arguments": {
                    "query": "beta",
                    "workspaceRoot": second.path()
                }
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["isError"], true);
        assert_eq!(state.active_root, first_root);
    }

    #[test]
    fn rejects_parent_traversal_workspace_argument() {
        let first = tempfile::tempdir().unwrap();
        fs::create_dir(first.path().join("child")).unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        let mut state =
            ServerState::with_allowed_roots(first.path(), [canonical(first.path())]).unwrap();
        let first_root = state.active_root.clone();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_switch_workspace",
                "arguments": {
                    "workspaceRoot": first.path().join("child").join("..")
                }
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["isError"], true);
        assert_eq!(state.active_root, first_root);
    }

    #[test]
    fn bundle_inherits_and_overrides_workspace_root() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        fs::write(second.path().join("second.rs"), "fn beta() {}\n").unwrap();
        let mut state =
            ServerState::with_allowed_roots(first.path(), [canonical(second.path())]).unwrap();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_bundle",
                "arguments": {
                    "workspaceRoot": second.path(),
                    "ops": [
                        {"tool": "repolens_search", "arguments": {"query": "beta", "limit": 1}},
                        {"tool": "repolens_search", "arguments": {"workspaceRoot": first.path(), "query": "alpha", "limit": 1}}
                    ]
                }
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        let text = tool_text(response);
        assert!(text.contains("second.rs"));
        assert!(text.contains("first.rs"));
        assert_eq!(state.active_root, Some(canonical(first.path())));
    }

    #[test]
    fn failed_tool_call_restores_active_root() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.rs"), "fn alpha() {}\n").unwrap();
        fs::write(second.path().join("second.rs"), "fn beta() {}\n").unwrap();
        let mut state =
            ServerState::with_allowed_roots(first.path(), [canonical(second.path())]).unwrap();
        let first_root = state.active_root.clone();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "repolens_search",
                "arguments": {"workspaceRoot": second.path()}
            }
        });
        let response = handle_line(&mut state, &request.to_string());

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["isError"], true);
        assert_eq!(state.active_root, first_root);
    }

    #[test]
    fn cache_is_bounded() {
        let first = tempfile::tempdir().unwrap();
        fs::write(first.path().join("root.rs"), "fn root() {}\n").unwrap();
        let mut dirs = Vec::new();
        let mut allowed = Vec::new();
        for index in 0..(MAX_CACHED_ROOTS + 3) {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                dir.path().join("file.rs"),
                format!("fn item_{index}() {{}}\n"),
            )
            .unwrap();
            allowed.push(canonical(dir.path()));
            dirs.push(dir);
        }
        let mut state = ServerState::with_allowed_roots(first.path(), allowed.clone()).unwrap();

        for root in allowed {
            state.switch_root(root.as_std_path()).unwrap();
        }

        assert!(state.indexes.len() <= MAX_CACHED_ROOTS);
    }

    #[test]
    fn switch_workspace_schema_requires_root_or_alias() {
        let switch_tool = tools()
            .into_iter()
            .find(|tool| tool["name"] == "repolens_switch_workspace")
            .unwrap();

        assert!(switch_tool["inputSchema"]["anyOf"].is_array());
    }
}
