use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
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

#[derive(Default)]
struct PendingChanges {
    reconcile: bool,
    rebuild: bool,
    fallback: bool,
    paths: BTreeSet<std::path::PathBuf>,
}

/// One bounded event accumulator per cached MCP workspace. Index mutation stays
/// on the request thread; callbacks never touch snapshots or write to stdout.
pub struct LiveWatcher {
    _watcher: Option<RecommendedWatcher>,
    pending: Arc<Mutex<PendingChanges>>,
    last_check: std::time::Instant,
    last_rules_check: std::time::Instant,
    rule_stamps: Vec<Option<(SystemTime, u64)>>,
    rule_paths: Vec<std::path::PathBuf>,
}

impl LiveWatcher {
    pub fn new(root: &Utf8Path) -> Self {
        let pending = Arc::new(Mutex::new(PendingChanges {
            reconcile: true, // Also catch edits made while the server was stopped.
            ..Default::default()
        }));
        let events = pending.clone();
        let watched_root = root.to_owned();
        let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let mut pending = events.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                    if event.need_rescan() {
                        pending.rebuild = true;
                    }
                    for path in event.paths {
                        if is_ignored_path(&watched_root, &path)
                            && path != watched_root.join(".git/info/exclude").as_std_path()
                        {
                            continue;
                        }
                        let rule = matches!(
                            path.file_name().and_then(|s| s.to_str()),
                            Some(".gitignore" | ".ignore" | ".repolensrc.toml")
                        ) || path.ends_with(".git/info/exclude");
                        if rule {
                            pending.rebuild = true;
                        } else if !is_ignored_path(&watched_root, &path) {
                            if pending.paths.len() < 4096 {
                                pending.paths.insert(path);
                            } else {
                                pending.rebuild = true;
                            }
                        }
                    }
                }
                Err(_) => {
                    pending.rebuild = true;
                    pending.fallback = true;
                }
                _ => {}
            }
        })
        .and_then(|mut watcher| {
            watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        let watcher = match watcher {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("MCP watch failed for {root}: {error}; using 2s polling");
                None
            }
        };
        // Inherited ignore files live outside the recursively watched workspace.
        let rule_paths = root
            .ancestors()
            .flat_map(|dir| {
                [
                    dir.join(".ignore").into_std_path_buf(),
                    dir.join(".gitignore").into_std_path_buf(),
                    dir.join(".git/info/exclude").into_std_path_buf(),
                ]
            })
            .collect::<Vec<_>>();
        let rule_stamps = Self::rule_stamps(&rule_paths);
        Self {
            _watcher: watcher,
            pending,
            last_check: std::time::Instant::now(),
            last_rules_check: std::time::Instant::now(),
            rule_stamps,
            rule_paths,
        }
    }

    fn rule_stamps(paths: &[std::path::PathBuf]) -> Vec<Option<(SystemTime, u64)>> {
        paths
            .iter()
            .map(|path| {
                fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok().map(|time| (time, m.len())))
            })
            .collect()
    }

    pub fn refresh(&mut self, index: &mut ProjectIndex) -> Result<()> {
        let mut changes = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *pending)
        };
        if changes.fallback {
            eprintln!("MCP watch lost events for {}; using 2s polling", index.root);
            self._watcher = None;
        }
        if self._watcher.is_none() && self.last_check.elapsed() >= Duration::from_secs(2) {
            changes.reconcile = true;
        }
        if self.last_rules_check.elapsed() >= Duration::from_secs(2) {
            let stamps = Self::rule_stamps(&self.rule_paths);
            changes.rebuild |= stamps != self.rule_stamps;
            self.rule_stamps = stamps;
            self.last_rules_check = std::time::Instant::now();
        }
        if !changes.reconcile && !changes.rebuild && changes.paths.is_empty() {
            return Ok(());
        }
        self.last_check = std::time::Instant::now();
        let result = self.apply(index, &changes);
        if result.is_err() {
            // Never lose a failed batch (e.g. an editor temporarily locks a file).
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .rebuild = true;
        }
        result
    }

    fn apply(&self, index: &mut ProjectIndex, changes: &PendingChanges) -> Result<()> {
        if changes.rebuild {
            *index = ProjectIndex::build(index.root.as_std_path())?;
            return Ok(());
        }
        // Ordinary file notifications need neither a repository signature scan
        // nor reads of unrelated binary assets. Directory/ambiguous events,
        // startup and polling retain the conservative reconciliation below.
        if !changes.reconcile
            && changes.paths.len() <= 32
            && changes.paths.iter().all(|path| {
                relative_utf8(&index.root, path).is_some_and(|relative| {
                    (path.is_file() || index.file_by_path(&relative).is_some())
                        && !index
                            .files
                            .iter()
                            .any(|file| file.path != relative && file.path.starts_with(&relative))
                })
            })
        {
            for path in &changes.paths {
                let relative = relative_utf8(&index.root, path).expect("validated relative path");
                if path.is_file() {
                    index.upsert_file(&relative)?;
                } else {
                    index.remove_file(&relative)?;
                }
            }
            return Ok(());
        }
        let signature = file_signature(&index.root)?;
        let indexed = index
            .files
            .iter()
            .map(|f| (f.path.as_str(), (f.bytes, f.mtime_ms)))
            .collect::<BTreeMap<_, _>>();
        let removed = index
            .files
            .iter()
            .filter(|file| !signature.contains_key(file.path.as_str()))
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let changed = signature
            .iter()
            .filter(|(path, signature)| {
                let relative = Utf8PathBuf::from(path.as_str());
                changes
                    .paths
                    .contains(index.root.join(&relative).as_std_path())
                    || indexed.get(path.as_str()) != Some(*signature)
            })
            .map(|(path, _)| Utf8PathBuf::from(path))
            .collect::<Vec<_>>();
        // The scanner also lists small binary assets, but the index omits them.
        // Do not mistake those existing assets for newly added source files on
        // every event (which would repeatedly rebuild the entire repository).
        let mut source_changes = Vec::new();
        for path in changed {
            if !indexed.contains_key(path.as_str()) {
                let bytes = fs::read(index.root.join(&path))?;
                if scanner::looks_binary(&bytes) {
                    continue;
                }
            }
            source_changes.push(path);
        }
        let changed = source_changes;
        if removed.len() + changed.len() > 32 {
            *index = ProjectIndex::build(index.root.as_std_path())?;
        } else {
            for path in removed {
                index.remove_file(&path)?;
            }
            for path in changed {
                index.upsert_file(&path)?;
            }
        }
        Ok(())
    }
}

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
                let index = reindex_event_paths(root, &event.paths)?;
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
            let index = reindex_poll_changes(root, &changes)?;
            let state = record_poll_changes(root, changes)?;
            println!("sequence: {} files: {}", state.sequence, index.files.len());
        }
        previous = current;
    }
}

fn reindex_event_paths(root: &Utf8Path, paths: &[std::path::PathBuf]) -> Result<ProjectIndex> {
    if paths.iter().any(|path| {
        matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some(".gitignore" | ".ignore" | ".repolensrc.toml")
        )
    }) {
        return rebuild(root);
    }
    let mut index = snapshot::load_or_build(root.as_std_path())?;
    // Use the same eligibility rules as a full index: hidden/sensitive files,
    // ignore files, nested generated directories, and the size limit all apply.
    let config = Config::load(root.as_std_path())?;
    let eligible = scanner::source_files(root.as_std_path(), &config)?
        .into_iter()
        .filter_map(|path| relative_utf8(root, &path))
        .collect::<BTreeSet<_>>();
    let excluded = index
        .files
        .iter()
        .filter(|file| !eligible.contains(&file.path))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    for path in excluded {
        index.remove_file(&path)?;
    }
    for path in paths.iter().filter(|path| !is_ignored_path(root, path)) {
        let Some(relative) = relative_utf8(root, path) else {
            return rebuild(root);
        };
        if eligible.contains(&relative) {
            index.upsert_file(&relative)?;
        } else if path.is_file() {
            // It exists but the scanner excludes it.
            continue;
        } else if index.file_by_path(&relative).is_some() {
            index.remove_file(&relative)?;
        } else {
            return rebuild(root);
        }
    }
    snapshot::save(&index)?;
    Ok(index)
}

fn reindex_poll_changes(root: &Utf8Path, changes: &[ChangeRecord]) -> Result<ProjectIndex> {
    let mut index = snapshot::load_or_build(root.as_std_path())?;
    for change in changes {
        let relative = Utf8PathBuf::from(&change.path);
        match change.kind.as_str() {
            "create" | "modify" => index.upsert_file(&relative)?,
            "remove" => index.remove_file(&relative)?,
            _ => return rebuild(root),
        }
    }
    snapshot::save(&index)?;
    Ok(index)
}

fn rebuild(root: &Utf8Path) -> Result<ProjectIndex> {
    let index = ProjectIndex::build(root.as_std_path())?;
    snapshot::save(&index)?;
    Ok(index)
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
        .is_some_and(|relative| {
            relative.components().any(|component| {
                let name = component.as_os_str().to_string_lossy();
                SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
            })
        })
}

fn display_relative(root: &Utf8Path, path: &Path) -> String {
    let relative = path.strip_prefix(root.as_std_path()).unwrap_or(path);
    Utf8PathBuf::from_path_buf(relative.to_path_buf())
        .map(|path| path.to_string())
        .unwrap_or_else(|_| relative.to_string_lossy().to_string())
}

fn relative_utf8(root: &Utf8Path, path: &Path) -> Option<Utf8PathBuf> {
    let relative = path.strip_prefix(root.as_std_path()).ok()?;
    Utf8PathBuf::from_path_buf(relative.to_path_buf()).ok()
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

    use super::{diff_signatures, read_state, record_event, reindex_poll_changes};

    #[test]
    fn directory_replaced_by_file_removes_indexed_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("replaced.py");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("child.py"), "old_child_token\n").unwrap();
        let mut index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        let watcher = super::LiveWatcher::new(&index.root);
        std::fs::remove_file(path.join("child.py")).unwrap();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "new_file_token\n").unwrap();
        watcher
            .apply(
                &mut index,
                &super::PendingChanges {
                    paths: [path].into_iter().collect(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].path, "replaced.py");
        assert_eq!(
            crate::search::search_hits(&index, "new_file_token", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn targeted_file_events_preserve_exclusions_and_remove_binary_or_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".ignore"), "ignored.py\n").unwrap();
        std::fs::write(
            temp.path().join(".repolensrc.toml"),
            "max_file_size = \"128\"\n",
        )
        .unwrap();
        let mut index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        let watcher = super::LiveWatcher::new(&index.root);
        for name in ["main.py", "ignored.py", ".hidden.py", ".env"] {
            let path = temp.path().join(name);
            std::fs::write(&path, "def unique_live_symbol(): pass\n").unwrap();
            watcher
                .apply(
                    &mut index,
                    &super::PendingChanges {
                        paths: [path].into_iter().collect(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].path, "main.py");
        let path = temp.path().join("main.py");
        for content in [
            vec![b'x'; 256],
            b"binary\0content".to_vec(),
            b"def restored_symbol(): pass\n".to_vec(),
        ] {
            std::fs::write(&path, content).unwrap();
            watcher
                .apply(
                    &mut index,
                    &super::PendingChanges {
                        paths: [path.clone()].into_iter().collect(),
                        ..Default::default()
                    },
                )
                .unwrap();
            let full = crate::index::ProjectIndex::build(temp.path()).unwrap();
            assert_eq!(
                index.files.iter().map(|f| &f.path).collect::<Vec<_>>(),
                full.files.iter().map(|f| &f.path).collect::<Vec<_>>()
            );
            assert_eq!(
                serde_json::to_value(crate::search::search_hits(&index, "symbol", 100).unwrap())
                    .unwrap(),
                serde_json::to_value(crate::search::search_hits(&full, "symbol", 100).unwrap())
                    .unwrap()
            );
        }
    }

    #[test]
    fn live_polling_and_inherited_ignore_changes_refresh_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("workspace");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "before\n").unwrap();
        let mut index = crate::index::ProjectIndex::build(&dir).unwrap();
        let mut watcher = super::LiveWatcher::new(&index.root);
        watcher.refresh(&mut index).unwrap();
        watcher._watcher = None;
        std::fs::write(dir.join("new.py"), "polling_unique_token\n").unwrap();
        watcher.last_check -= std::time::Duration::from_secs(3);
        watcher.refresh(&mut index).unwrap();
        assert_eq!(
            crate::search::search_hits(&index, "polling_unique_token", 10)
                .unwrap()
                .len(),
            1
        );
        std::fs::write(temp.path().join(".ignore"), "new.py\n").unwrap();
        watcher.last_rules_check -= std::time::Duration::from_secs(3);
        watcher.refresh(&mut index).unwrap();
        assert_eq!(
            crate::search::search_hits(&index, "polling_unique_token", 10)
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_file(temp.path().join(".ignore")).unwrap();
        watcher.last_rules_check -= std::time::Duration::from_secs(3);
        watcher.refresh(&mut index).unwrap();
        assert_eq!(
            crate::search::search_hits(&index, "polling_unique_token", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn live_refresh_retries_failed_batches() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.py"), "before\n").unwrap();
        let mut index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        let mut watcher = super::LiveWatcher::new(&index.root);
        std::fs::write(temp.path().join(".repolensrc.toml"), "invalid = [").unwrap();
        assert!(watcher.refresh(&mut index).is_err());
        std::fs::remove_file(temp.path().join(".repolensrc.toml")).unwrap();
        std::fs::write(temp.path().join("a.py"), "recovered_unique_token\n").unwrap();
        watcher.refresh(&mut index).unwrap();
        assert_eq!(
            crate::search::search_hits(&index, "recovered_unique_token", 10)
                .unwrap()
                .len(),
            1
        );
    }

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

    #[test]
    fn polling_modify_refreshes_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn before() {}\n").unwrap();
        let index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        crate::snapshot::save(&index).unwrap();

        std::fs::write(temp.path().join("main.rs"), "fn after() {}\n").unwrap();
        reindex_poll_changes(
            &root,
            &[super::ChangeRecord {
                path: "main.rs".to_string(),
                kind: "modify".to_string(),
            }],
        )
        .unwrap();
        let loaded = crate::snapshot::load_or_build(temp.path()).unwrap();

        assert!(loaded.symbols.iter().any(|symbol| symbol.name == "after"));
        assert!(!loaded.symbols.iter().any(|symbol| symbol.name == "before"));
    }

    #[test]
    fn notify_respects_scanner_exclusions_and_removes_newly_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        crate::snapshot::save(&index).unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
        for name in [".env", "ignored.rs", ".hidden.rs", "large.rs"] {
            let content = if name == "large.rs" {
                "x".repeat(1024 * 1024 + 1)
            } else {
                "fn excluded() {}".to_owned()
            };
            std::fs::write(temp.path().join(name), content).unwrap();
            let updated = super::reindex_event_paths(&root, &[temp.path().join(name)]).unwrap();
            assert!(
                updated
                    .file_by_path(&camino::Utf8PathBuf::from(name))
                    .is_none(),
                "{name}"
            );
        }
        std::fs::write(temp.path().join("main.rs"), "x".repeat(1024 * 1024 + 1)).unwrap();
        let updated = super::reindex_event_paths(&root, &[temp.path().join("main.rs")]).unwrap();
        assert!(updated.files.is_empty());
    }

    #[test]
    fn notify_rebuilds_when_ignore_rules_change() {
        let temp = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        crate::snapshot::save(&index).unwrap();
        let ignore = temp.path().join(".gitignore");
        std::fs::write(&ignore, "main.rs\n").unwrap();
        assert!(
            super::reindex_event_paths(&root, std::slice::from_ref(&ignore))
                .unwrap()
                .files
                .is_empty()
        );
        std::fs::write(&ignore, "").unwrap();
        assert_eq!(
            super::reindex_event_paths(&root, &[ignore])
                .unwrap()
                .files
                .len(),
            1
        );
        assert!(super::is_ignored_path(
            &root,
            &temp.path().join("packages/app/node_modules/file.rs")
        ));
    }

    #[test]
    fn polling_create_and_remove_update_index() {
        let temp = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let index = crate::index::ProjectIndex::build(temp.path()).unwrap();
        crate::snapshot::save(&index).unwrap();

        std::fs::write(temp.path().join("added.rs"), "fn added() {}\n").unwrap();
        reindex_poll_changes(
            &root,
            &[super::ChangeRecord {
                path: "added.rs".to_string(),
                kind: "create".to_string(),
            }],
        )
        .unwrap();
        let loaded = crate::snapshot::load_or_build(temp.path()).unwrap();
        assert!(loaded.symbols.iter().any(|symbol| symbol.name == "added"));

        std::fs::remove_file(temp.path().join("added.rs")).unwrap();
        reindex_poll_changes(
            &root,
            &[super::ChangeRecord {
                path: "added.rs".to_string(),
                kind: "remove".to_string(),
            }],
        )
        .unwrap();
        let loaded = crate::snapshot::load_or_build(temp.path()).unwrap();
        assert!(!loaded.symbols.iter().any(|symbol| symbol.name == "added"));
    }
}
