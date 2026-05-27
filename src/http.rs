use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cli::EditOpArg;
use crate::index::ProjectIndex;
use crate::{deps, edit, read, search, snapshot, symbols, watcher};

#[derive(Clone)]
struct AppState {
    index: Arc<ProjectIndex>,
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
        let index = snapshot::load_or_build(root)?;
        let state = AppState {
            index: Arc::new(index),
        };
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
        .with_state(state)
}

async fn status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "root": state.index.root,
        "files": state.index.files.len(),
        "words": state.index.words.len(),
        "trigrams": state.index.trigrams.len(),
        "symbols": state.index.symbols.len(),
        "symbol_names": state.index.symbols_by_name.len(),
        "deps_files": state.index.deps.len(),
        "deps_forward": state.index.deps_forward.len(),
        "deps_reverse": state.index.deps_reverse.len()
    }))
}

async fn snapshot_info(State(state): State<AppState>) -> Json<Value> {
    Json(json!(snapshot::info(&state.index)))
}

async fn tree(State(state): State<AppState>, Query(query): Query<LimitQuery>) -> Json<Value> {
    let limit = query.limit.unwrap_or(200);
    Json(json!(
        state
            .index
            .files
            .iter()
            .take(limit)
            .map(|file| json!({"path": file.path, "lines": file.lines, "bytes": file.bytes, "hash": file.hash}))
            .collect::<Vec<_>>()
    ))
}

async fn search_route(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(search::search_hits(
        &state.index,
        &query.q,
        query.limit.unwrap_or(20)
    )?)))
}

async fn word(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Json<Value> {
    Json(json!(search::word_paths(
        &state.index,
        &query.q,
        query.limit.unwrap_or(20)
    )))
}

async fn read_route(
    State(state): State<AppState>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = Utf8PathBuf::from(query.path);
    Ok(Json(json!({
        "path": path,
        "text": read::read_text(
            state.index.root.as_std_path(),
            &path,
            query.lines.as_deref(),
            query.max_bytes,
            query.hash.as_deref(),
        )?
    })))
}

async fn outline(State(state): State<AppState>, Query(query): Query<PathQuery>) -> Json<Value> {
    Json(json!(symbols::outline(
        &state.index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn symbol(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Json<Value> {
    Json(json!(symbols::find(
        &state.index,
        &query.q,
        query.limit.unwrap_or(20)
    )))
}

async fn deps_route(State(state): State<AppState>, Query(query): Query<PathQuery>) -> Json<Value> {
    Json(json!(deps::deps_for_file(
        &state.index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn rdeps(State(state): State<AppState>, Query(query): Query<PathQuery>) -> Json<Value> {
    Json(json!(deps::reverse_deps_for_file(
        &state.index,
        &Utf8PathBuf::from(query.path)
    )))
}

async fn changes(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(watcher::read_state(&state.index.root)?)))
}

async fn edit_route(
    State(state): State<AppState>,
    Json(request): Json<EditRequest>,
) -> Result<Json<Value>, ApiError> {
    let op = match request.op.as_str() {
        "replace" => EditOpArg::Replace,
        "insert" => EditOpArg::Insert,
        "delete" => EditOpArg::Delete,
        _ => return Err(ApiError(anyhow!("invalid edit op"))),
    };
    Ok(Json(json!(edit::apply(
        state.index.root.as_std_path(),
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

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    #[test]
    fn loopback_only_accepts_localhost() {
        assert!("127.0.0.1".parse::<IpAddr>().unwrap().is_loopback());
        assert!("::1".parse::<IpAddr>().unwrap().is_loopback());
        assert!(!"0.0.0.0".parse::<IpAddr>().unwrap().is_loopback());
    }
}
