use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Result, anyhow, bail};
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cli::EditOpArg;
use crate::index::ProjectIndex;
use crate::{deps, edit, read, search, snapshot, symbols, watcher};

#[derive(Clone)]
struct AppState {
    cache: Arc<Mutex<CachedIndex>>,
}

struct CachedIndex {
    index: Arc<ProjectIndex>,
    stamp: SnapshotStamp,
}

type SnapshotStamp = [Option<(SystemTime, u64)>; 3];

fn snapshot_stamp(index: &ProjectIndex) -> Result<SnapshotStamp> {
    let mut stamp = [None, None, None];
    for (slot, path) in stamp.iter_mut().zip([
        snapshot::index_path(&index.root),
        snapshot::binary_index_path(&index.root),
        index.root.join(".repolensrc.toml"),
    ]) {
        match std::fs::metadata(path) {
            Ok(metadata) => *slot = Some((metadata.modified()?, metadata.len())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(stamp)
}

impl AppState {
    fn new(root: &Path) -> Result<Self> {
        let index = Arc::new(snapshot::load_or_build(root)?);
        let stamp = snapshot_stamp(&index)?;
        Ok(Self {
            cache: Arc::new(Mutex::new(CachedIndex { index, stamp })),
        })
    }

    fn current_index(&self) -> Result<Arc<ProjectIndex>> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("index cache lock poisoned"))?;
        let stamp = snapshot_stamp(&cache.index)?;
        if stamp != cache.stamp {
            let index = snapshot::load_or_build(cache.index.root.as_std_path())?;
            cache.index = Arc::new(index);
            // A save during loading must trigger another refresh on the next request.
            cache.stamp = stamp;
        }
        Ok(Arc::clone(&cache.index))
    }
}

async fn refresh_index(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match state.current_index() {
        Ok(index) => {
            request.extensions_mut().insert(index);
            next.run(request).await
        }
        Err(error) => ApiError(error).into_response(),
    }
}

#[derive(Debug)]
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": self.0.to_string()})),
        )
            .into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

#[derive(Debug, Deserialize)]
struct LimitQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PathQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct ReadQuery {
    path: String,
    lines: Option<String>,
    max_bytes: Option<usize>,
    hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EditRequest {
    path: String,
    op: String,
    start: usize,
    end: Option<usize>,
    content: Option<String>,
    hash: String,
}

pub fn serve(root: &Path, host: &str, port: u16) -> Result<()> {
    let host: IpAddr = host.parse()?;
    if !host.is_loopback() {
        bail!("HTTP server only binds loopback addresses");
    }

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let state = AppState::new(root)?;
        let app = router(state);
        let addr = SocketAddr::new(host, port);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        println!("serving http://{addr}");
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui))
        .route("/status", get(status))
        .route("/snapshot", get(snapshot_info))
        .route("/tree", get(tree))
        .route("/search", get(search_route))
        .route("/word", get(word))
        .route("/read", get(read_route))
        .route("/outline", get(outline))
        .route("/symbol", get(symbol))
        .route("/deps", get(deps_route))
        .route("/rdeps", get(rdeps))
        .route("/changes", get(changes))
        .route("/edit", post(edit_route))
        .layer(middleware::from_fn_with_state(state, refresh_index))
}

async fn ui() -> Html<&'static str> {
    Html(UI_HTML)
}

async fn status(Extension(index): Extension<Arc<ProjectIndex>>) -> Json<Value> {
    Json(json!({
        "root": index.root,
        "files": index.files.len(),
        "words": index.words.len(),
        "trigrams": index.trigrams.len(),
        "symbols": index.symbols.len(),
        "symbol_names": index.symbols_by_name.len(),
        "deps_files": index.deps.len(),
        "deps_forward": index.deps_forward.len(),
        "deps_reverse": index.deps_reverse.len()
    }))
}

async fn snapshot_info(Extension(index): Extension<Arc<ProjectIndex>>) -> Json<Value> {
    Json(json!(snapshot::info(&index)))
}

async fn tree(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<LimitQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(200);
    Json(json!(
        index
            .files
            .iter()
            .take(limit)
            .map(|file| json!({"path": file.path, "lines": file.lines, "bytes": file.bytes, "hash": file.hash}))
            .collect::<Vec<_>>()
    ))
}

async fn search_route(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(search::search_hits(
        &index,
        &query.q,
        query.limit.unwrap_or(20)
    )?)))
}

async fn word(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<SearchQuery>,
) -> Json<Value> {
    Json(json!(search::word_paths(
        &index,
        &query.q,
        query.limit.unwrap_or(20)
    )))
}

async fn read_route(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = Utf8PathBuf::from(query.path);
    Ok(Json(json!({
        "path": path,
        "text": read::read_text(
            index.root.as_std_path(),
            &path,
            query.lines.as_deref(),
            query.max_bytes,
            query.hash.as_deref(),
        )?
    })))
}

async fn outline(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<PathQuery>,
) -> Json<Value> {
    Json(json!(symbols::outline(
        &index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn symbol(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<SearchQuery>,
) -> Json<Value> {
    Json(json!(symbols::find(
        &index,
        &query.q,
        query.limit.unwrap_or(20)
    )))
}

async fn deps_route(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<PathQuery>,
) -> Json<Value> {
    Json(json!(deps::deps_for_file(
        &index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn rdeps(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Query(query): Query<PathQuery>,
) -> Json<Value> {
    Json(json!(deps::reverse_deps_for_file(
        &index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn changes(Extension(index): Extension<Arc<ProjectIndex>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(watcher::read_state(&index.root)?)))
}

async fn edit_route(
    Extension(index): Extension<Arc<ProjectIndex>>,
    Json(request): Json<EditRequest>,
) -> Result<Json<Value>, ApiError> {
    let op = match request.op.as_str() {
        "replace" => EditOpArg::Replace,
        "insert" => EditOpArg::Insert,
        "delete" => EditOpArg::Delete,
        _ => return Err(ApiError(anyhow!("invalid edit op"))),
    };
    Ok(Json(json!(edit::apply(
        index.root.as_std_path(),
        &Utf8PathBuf::from(request.path),
        op,
        request.start,
        request.end,
        request.content.as_deref(),
        &request.hash,
    )?)))
}

#[allow(dead_code)]
fn default_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4177)
}

#[allow(dead_code)]
fn ipv6_loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
}

const UI_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>RepoLens</title>
  <style>
    :root { --bg:#f6f7f9; --panel:#fff; --text:#17202a; --muted:#5c6670; --line:#d9dee5; --accent:#166534; --accent2:#1d4ed8; --code:#111827; }
    * { box-sizing: border-box; }
    body { margin:0; background:var(--bg); color:var(--text); font:14px/1.45 system-ui,-apple-system,Segoe UI,sans-serif; }
    header { display:flex; justify-content:space-between; gap:16px; padding:14px 20px; border-bottom:1px solid var(--line); background:var(--panel); }
    h1 { margin:0; font-size:18px; }
    main { display:grid; grid-template-columns:320px minmax(0,1fr); gap:16px; padding:16px; max-width:1500px; margin:0 auto; }
    section { background:var(--panel); border:1px solid var(--line); border-radius:8px; min-width:0; }
    .side,.work { display:grid; gap:16px; align-content:start; }
    .head { display:flex; align-items:center; justify-content:space-between; gap:8px; padding:10px 12px; border-bottom:1px solid var(--line); font-weight:650; }
    .body { padding:12px; }
    .metrics { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; }
    .metric { border:1px solid var(--line); border-radius:6px; padding:8px; min-height:58px; }
    .metric span,label,.meta { color:var(--muted); font-size:12px; }
    .metric strong { display:block; font-size:20px; margin-top:2px; }
    label { display:block; margin-bottom:5px; }
    input,select,button { height:34px; border:1px solid var(--line); border-radius:6px; background:#fff; color:var(--text); font:inherit; }
    input,select { width:100%; padding:0 9px; }
    button { padding:0 11px; cursor:pointer; background:#f8fafc; white-space:nowrap; }
    button.primary { background:var(--accent); border-color:var(--accent); color:#fff; }
    .row { display:flex; gap:8px; align-items:end; }
    .row>* { flex:1; min-width:0; }
    .row button { flex:0 0 auto; }
    .list { display:grid; gap:4px; max-height:360px; overflow:auto; }
    .item { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:8px; padding:6px 8px; border-radius:5px; border:1px solid transparent; background:#fff; text-align:left; height:auto; min-height:34px; }
    .item:hover { border-color:var(--line); background:#f8fafc; }
    .path { overflow-wrap:anywhere; font-family:ui-monospace,SFMono-Regular,Consolas,monospace; }
    pre { margin:0; min-height:520px; max-height:680px; overflow:auto; padding:12px; border-radius:6px; background:var(--code); color:#f8fafc; font:12px/1.55 ui-monospace,SFMono-Regular,Consolas,monospace; white-space:pre-wrap; }
    .tabs { display:flex; gap:6px; }
    .tabs button { height:30px; }
    .tabs button.active { color:#fff; background:var(--accent2); border-color:var(--accent2); }
    @media (max-width:900px) { main { grid-template-columns:1fr; padding:10px; } header { flex-direction:column; } }
  </style>
</head>
<body>
  <header><h1>RepoLens</h1><div id="root" class="meta"></div></header>
  <main>
    <div class="side">
      <section><div class="head">Status <button id="refresh">Refresh</button></div><div class="body"><div id="metrics" class="metrics"></div></div></section>
      <section><div class="head">Files</div><div class="body"><div class="row"><div><label>Limit</label><input id="treeLimit" value="100"></div><button id="loadTree">Load</button></div><div id="tree" class="list" style="margin-top:10px"></div></div></section>
    </div>
    <div class="work">
      <section><div class="head">Search</div><div class="body"><div class="row"><div><label>Query</label><input id="query" placeholder="ProjectIndex"></div><div><label>Mode</label><select id="mode"><option value="search">search</option><option value="symbol">symbol</option><option value="word">word</option></select></div><button id="runSearch" class="primary">Run</button></div><div id="results" class="list" style="margin-top:10px"></div></div></section>
      <section><div class="head"><span id="selected">No file selected</span><div class="tabs"><button data-view="read" class="active">Read</button><button data-view="outline">Outline</button><button data-view="deps">Deps</button><button data-view="rdeps">Rdeps</button></div></div><div class="body"><div class="row" style="margin-bottom:10px"><div><label>Lines</label><input id="lines" placeholder="1-80"></div><button id="loadView">Load</button></div><pre id="viewer"></pre></div></section>
    </div>
  </main>
  <script>
    const $ = id => document.getElementById(id); let selectedPath = ""; let currentView = "read";
    async function api(path) { const res = await fetch(path); if (!res.ok) throw new Error(await res.text()); return res.json(); }
    function pretty(value) { return JSON.stringify(value, null, 2); }
    function setViewer(value) { $("viewer").textContent = typeof value === "string" ? value : pretty(value); }
    function escapeHtml(text) { return String(text).replace(/[&<>"']/g, c => ({ "&":"&amp;", "<":"&lt;", ">":"&gt;", '"':"&quot;", "'":"&#39;" }[c])); }
    function fileButton(file) { const el = document.createElement("button"); el.className = "item"; el.innerHTML = `<span class="path">${escapeHtml(file.path)}</span><span class="meta">${file.lines} lines</span>`; el.onclick = () => selectFile(file.path); return el; }
    function resultButton(item) { const el = document.createElement("button"); el.className = "item"; const path = item.path || item; const info = item.line ? `${item.line}` : ""; el.innerHTML = `<span><span class="path">${escapeHtml(path)}</span><br><span class="meta">${escapeHtml(item.text || "")}</span></span><span class="meta">${info}</span>`; el.onclick = () => selectFile(path); return el; }
    async function loadStatus() { const data = await api("/status"); $("root").textContent = data.root; $("metrics").innerHTML = ""; for (const key of ["files","symbols","words","deps_files"]) { const div = document.createElement("div"); div.className = "metric"; div.innerHTML = `<span>${key}</span><strong>${data[key]}</strong>`; $("metrics").appendChild(div); } }
    async function loadTree() { const data = await api(`/tree?limit=${encodeURIComponent($("treeLimit").value || "100")}`); $("tree").replaceChildren(...data.map(fileButton)); }
    async function runSearch() { const q = $("query").value.trim(); if (!q) return; const mode = $("mode").value; const data = await api(`/${mode}?q=${encodeURIComponent(q)}&limit=50`); $("results").replaceChildren(...data.map(resultButton)); }
    async function selectFile(path) { selectedPath = path; $("selected").textContent = path; await loadView(); }
    async function loadView() { if (!selectedPath) return; if (currentView === "read") { const lines = $("lines").value.trim(); const data = await api(`/read?path=${encodeURIComponent(selectedPath)}${lines ? `&lines=${encodeURIComponent(lines)}` : ""}`); setViewer(data.text); } else { setViewer(await api(`/${currentView}?path=${encodeURIComponent(selectedPath)}`)); } }
    document.querySelectorAll("[data-view]").forEach(button => { button.onclick = () => { document.querySelectorAll("[data-view]").forEach(b => b.classList.remove("active")); button.classList.add("active"); currentView = button.dataset.view; loadView(); }; });
    $("refresh").onclick = loadStatus; $("loadTree").onclick = loadTree; $("runSearch").onclick = runSearch; $("loadView").onclick = loadView; $("query").onkeydown = e => { if (e.key === "Enter") runSearch(); };
    loadStatus().then(loadTree).catch(err => setViewer(String(err)));
  </script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{AppState, EditRequest, UI_HTML, edit_route};
    use crate::{index::ProjectIndex, search, snapshot};
    use axum::{Extension, Json};
    use std::{fs, sync::Arc};

    #[test]
    fn reloads_external_snapshot_and_reuses_unchanged_index() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn old_name() {}\n").unwrap();
        snapshot::save(&ProjectIndex::build(temp.path()).unwrap()).unwrap();
        let state = AppState::new(temp.path()).unwrap();
        let before = state.current_index().unwrap();
        assert!(Arc::ptr_eq(&before, &state.current_index().unwrap()));
        fs::write(temp.path().join("second.rs"), "fn external_addition() {}\n").unwrap();
        snapshot::save(&ProjectIndex::build(temp.path()).unwrap()).unwrap();
        let after = state.current_index().unwrap();
        assert_eq!(after.files.len(), 2);
        assert!(!search::word_paths(&after, "external_addition", 20).is_empty());
        assert!(Arc::ptr_eq(&after, &state.current_index().unwrap()));
    }

    #[test]
    fn cached_index_reloads_when_sensitive_access_is_revoked() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join(".repolensrc.toml");
        fs::write(&config, "allow_sensitive = true\ninclude_hidden = true\n").unwrap();
        fs::write(temp.path().join(".env"), "SYNTHETIC_MARKER=example\n").unwrap();
        snapshot::save(&ProjectIndex::build(temp.path()).unwrap()).unwrap();
        let state = AppState::new(temp.path()).unwrap();
        assert!(
            state
                .current_index()
                .unwrap()
                .files
                .iter()
                .any(|file| file.path == ".env")
        );
        fs::write(&config, "allow_sensitive = false\ninclude_hidden = true\n").unwrap();
        let index = state.current_index().unwrap();
        assert!(!index.files.iter().any(|file| file.path == ".env"));
        assert!(search::word_paths(&index, "SYNTHETIC_MARKER", 10).is_empty());
    }

    #[tokio::test]
    async fn edit_endpoint_refreshes_search_and_symbols() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.rs"), "fn before_edit() {}\n").unwrap();
        snapshot::save(&ProjectIndex::build(temp.path()).unwrap()).unwrap();
        let state = AppState::new(temp.path()).unwrap();
        let before = state.current_index().unwrap();
        let hash = before.files[0].hash.clone();
        let response = edit_route(
            Extension(before),
            Json(EditRequest {
                path: "main.rs".into(),
                op: "replace".into(),
                start: 1,
                end: None,
                content: Some("fn after_edit_with_longer_name() {}\n".into()),
                hash,
            }),
        )
        .await;
        assert!(response.is_ok());
        let after = state.current_index().unwrap();
        assert!(search::word_paths(&after, "before_edit", 20).is_empty());
        assert!(!search::word_paths(&after, "after_edit_with_longer_name", 20).is_empty());
        assert!(!crate::symbols::find(&after, "after_edit_with_longer_name", 20).is_empty());
    }

    #[test]
    fn loopback_only_accepts_localhost() {
        assert!("127.0.0.1".parse::<IpAddr>().unwrap().is_loopback());
        assert!("::1".parse::<IpAddr>().unwrap().is_loopback());
        assert!(!"0.0.0.0".parse::<IpAddr>().unwrap().is_loopback());
    }

    #[test]
    fn ui_contains_required_routes() {
        for route in ["/status", "/tree", "/read", "value=\"search\""] {
            assert!(UI_HTML.contains(route));
        }
    }
}
