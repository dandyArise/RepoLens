use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::index::ProjectIndex;
use crate::pathing::canonical_utf8;
use crate::scanner;
use crate::snapshot;

const SKIP_DIRS: &[&str] = &[
    ".repolens",
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    ".venv",
    "__pycache__",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchState {
    pub sequence: u64,
    pub updated_at_ms: u64,
    pub changes: Vec<ChangeRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub path: String,
    pub kind: String,
}

type FileSignature = BTreeMap<String, (u64, u64)>;

pub fn watch(root: &Path, poll: bool, interval_ms: u64) -> Result<()> {
    let root = canonical_utf8(root)?;
    if poll {
        return watch_polling(&root, interval_ms);
    }

    watch_notify(&root).or_else(|error| {
        eprintln!("notify watcher failed: {error}; falling back to polling");
        watch_polling(&root, interval_ms)
    })
}

fn watch_notify(root: &Utf8Path) -> Result<()> {
    let index = ProjectIndex::build(root.as_std_path())?;
    snapshot::save(&index)?;

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |result| {
        let _ = tx.send(result);
    })?;
    watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;

    println!("watching {}", root);
    for result in rx {
        match result {
            Ok(event) if should_handle_event(root, &event) => {
                let index = ProjectIndex::build(root.as_std_path())?;
                snapshot::save(&index)?;
                let state = record_event(root, &event)?;
                println!("sequence: {} files: {}", state.sequence, index.files.len());
            }
            Ok(_) => {}
            Err(error) => eprintln!("watch error: {error}"),
        }
    }

    Ok(())
}

fn watch_polling(root: &Utf8Path, interval_ms: u64) -> Result<()> {
    let index = ProjectIndex::build(root.as_std_path())?;
    snapshot::save(&index)?;
    let mut previous = file_signature(root)?;
    let interval = Duration::from_millis(interval_ms.max(100));

    println!("watching {} with polling", root);
    loop {
        thread::sleep(interval);
        let current = file_signature(root)?;
        let changes = diff_signatures(&previous, &current);
        if !changes.is_empty() {
            let index = ProjectIndex::build(root.as_std_path())?;
            snapshot::save(&index)?;
            let state = record_poll_changes(root, changes)?;
            println!("sequence: {} files: {}", state.sequence, index.files.len());
        }
        previous = current;
    }
}

pub fn print_changes(root: &Path) -> Result<()> {
    let root = canonical_utf8(root)?;
    let state = read_state(&root)?;
    println!("sequence: {}", state.sequence);
    println!("updated_at_ms: {}", state.updated_at_ms);
    for change in state.changes {
        println!("{}\t{}", change.kind, change.path);
    }
    Ok(())
}

pub fn print_hot(root: &Path, limit: usize) -> Result<()> {
    let root = canonical_utf8(root)?;
    let state = read_state(&root)?;
    for change in state.changes.into_iter().take(limit) {
        println!("{}", change.path);
    }
    Ok(())
}

pub fn read_state(root: &Utf8Path) -> Result<WatchState> {
    let path = state_path(root);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).with_context(|| format!("failed to parse {path}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WatchState::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {path}")),
    }
}

fn record_event(root: &Utf8Path, event: &Event) -> Result<WatchState> {
    let mut state = read_state(root)?;
    state.sequence += 1;
    state.updated_at_ms = now_ms();
    state.changes = event
        .paths
        .iter()
        .filter(|path| !is_ignored_path(root, path))
        .map(|path| ChangeRecord {
            path: display_relative(root, path),
            kind: format!("{:?}", event.kind),
        })
        .collect();
    write_state(root, &state)?;
    Ok(state)
}

fn record_poll_changes(root: &Utf8Path, changes: Vec<ChangeRecord>) -> Result<WatchState> {
    let mut state = read_state(root)?;
    state.sequence += 1;
    state.updated_at_ms = now_ms();
    state.changes = changes;
    write_state(root, &state)?;
    Ok(state)
}

fn write_state(root: &Utf8Path, state: &WatchState) -> Result<()> {
    let path = state_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    fs::write(&path, serde_json::to_string_pretty(state)?)
        .with_context(|| format!("failed to write {path}"))?;
    Ok(())
}

fn state_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".repolens").join("changes.json")
}

fn should_handle_event(root: &Utf8Path, event: &Event) -> bool {
    !event.paths.is_empty() && event.paths.iter().any(|path| !is_ignored_path(root, path))
}

fn file_signature(root: &Utf8Path) -> Result<FileSignature> {
    let config = Config::load(root.as_std_path())?;
    let mut signature = BTreeMap::new();
    for path in scanner::source_files(root.as_std_path(), &config)? {
        if is_ignored_path(root, &path) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        signature.insert(
            display_relative(root, &path),
            (metadata.len(), metadata_mtime_ms(&metadata)),
        );
    }
    Ok(signature)
}

fn diff_signatures(previous: &FileSignature, current: &FileSignature) -> Vec<ChangeRecord> {
    let mut paths: BTreeSet<&String> = previous.keys().collect();
    paths.extend(current.keys());
    paths
        .into_iter()
        .filter_map(|path| match (previous.get(path), current.get(path)) {
            (None, Some(_)) => Some(change(path, "create")),
            (Some(_), None) => Some(change(path, "remove")),
            (Some(before), Some(after)) if before != after => Some(change(path, "modify")),
            _ => None,
        })
        .collect()
}

fn change(path: &str, kind: &str) -> ChangeRecord {
    ChangeRecord {
        path: path.to_string(),
        kind: kind.to_string(),
    }
}

fn is_ignored_path(root: &Utf8Path, path: &Path) -> bool {
    path.strip_prefix(root.as_std_path())
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| {
            let name = component.as_os_str().to_string_lossy();
            SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
        })
}

fn display_relative(root: &Utf8Path, path: &Path) -> String {
    let relative = path.strip_prefix(root.as_std_path()).unwrap_or(path);
    Utf8PathBuf::from_path_buf(relative.to_path_buf())
        .map(|path| path.to_string())
        .unwrap_or_else(|_| relative.to_string_lossy().to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn metadata_mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use notify::{Event, EventKind};

    use super::{diff_signatures, read_state, record_event};

    #[test]
    fn records_watch_sequence() {
        let temp = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let event = Event {
            kind: EventKind::Any,
            paths: vec![temp.path().join("src").join("main.rs")],
            attrs: Default::default(),
        };

        record_event(&root, &event).unwrap();
        let state = read_state(&root).unwrap();

        assert_eq!(state.sequence, 1);
        assert_eq!(state.changes[0].path.replace('\\', "/"), "src/main.rs");
    }

    #[test]
    fn diffs_polling_signatures() {
        let mut before = std::collections::BTreeMap::new();
        before.insert("a.rs".to_string(), (1, 1));
        before.insert("b.rs".to_string(), (1, 1));
        let mut after = std::collections::BTreeMap::new();
        after.insert("a.rs".to_string(), (2, 2));
        after.insert("c.rs".to_string(), (1, 1));

        let changes = diff_signatures(&before, &after);
        assert!(
            changes
                .iter()
                .any(|change| change.path == "a.rs" && change.kind == "modify")
        );
        assert!(
            changes
                .iter()
                .any(|change| change.path == "b.rs" && change.kind == "remove")
        );
        assert!(
            changes
                .iter()
                .any(|change| change.path == "c.rs" && change.kind == "create")
        );
    }
}
