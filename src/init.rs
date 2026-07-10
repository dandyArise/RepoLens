use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::cli::InitTargetArg;
use crate::pathing::canonical_utf8;

pub fn enable(target: InitTargetArg, root: &Path) -> Result<()> {
    let root = canonical_utf8(root)?;
    let exe = std::env::current_exe().context("failed to locate current executable")?;

    if matches!(target, InitTargetArg::All | InitTargetArg::Codex) {
        let path = codex_config_path();
        write_codex_config(&path, &exe, root.as_std_path())?;
        println!("configured Codex: {}", path.display());
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Claude) {
        let path = claude_config_path();
        write_json_mcp_config(&path, &exe, root.as_std_path())?;
        println!("configured Claude: {}", path.display());
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Cursor) {
        let path = cursor_config_path();
        write_json_mcp_config(&path, &exe, root.as_std_path())?;
        println!("configured Cursor: {}", path.display());
    }

    Ok(())
}

pub fn disable(target: InitTargetArg) -> Result<()> {
    if matches!(target, InitTargetArg::All | InitTargetArg::Codex) {
        let path = codex_config_path();
        remove_codex_config(&path)?;
        println!("disabled Codex: {}", path.display());
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Claude) {
        let path = claude_config_path();
        remove_json_mcp_config(&path)?;
        println!("disabled Claude: {}", path.display());
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Cursor) {
        let path = cursor_config_path();
        remove_json_mcp_config(&path)?;
        println!("disabled Cursor: {}", path.display());
    }

    Ok(())
}

pub fn status(target: InitTargetArg) -> Result<()> {
    if matches!(target, InitTargetArg::All | InitTargetArg::Codex) {
        let path = codex_config_path();
        let project_count = codex_project_count(&path)?;
        println!(
            "Codex: {} ({})",
            codex_status_label(project_count),
            path.display()
        );
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Claude) {
        let path = claude_config_path();
        println!(
            "Claude: {} ({})",
            enabled_label(json_mcp_enabled(&path)?),
            path.display()
        );
    }

    if matches!(target, InitTargetArg::All | InitTargetArg::Cursor) {
        let path = cursor_config_path();
        println!(
            "Cursor: {} ({})",
            enabled_label(json_mcp_enabled(&path)?),
            path.display()
        );
    }

    Ok(())
}

fn codex_config_path() -> PathBuf {
    home_dir().join(".codex").join("config.toml")
}

fn claude_config_path() -> PathBuf {
    if cfg!(windows) {
        data_dir().join("Claude").join("claude_desktop_config.json")
    } else {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude_desktop_config.json")
    }
}

fn cursor_config_path() -> PathBuf {
    home_dir().join(".cursor").join("mcp.json")
}

fn write_codex_config(path: &Path, exe: &Path, root: &Path) -> Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let mut document = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", path.display()))?
    };

    if !document.contains_key("mcp_servers") {
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = document["mcp_servers"].as_table_mut().with_context(|| {
        format!(
            "failed to update {}: mcp_servers is not a table",
            path.display()
        )
    })?;

    migrate_legacy_codex_entry(servers);

    let matching_name = find_codex_server_for_root(servers, root);
    let server_name = matching_name.unwrap_or_else(|| unique_codex_server_name(servers, root));
    write_codex_server(servers, &server_name, exe, root);

    write_config_if_changed(path, &existing, document.to_string())
}

fn remove_codex_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let existing =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut document = existing
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    let Some(servers) = document.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return Ok(());
    };

    let names = repolens_codex_entries(servers)
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    for name in names {
        servers.remove(&name);
    }

    write_config_if_changed(path, &existing, document.to_string())
}

fn migrate_legacy_codex_entry(servers: &mut Table) {
    let Some(legacy_root) = servers.get("repolens").and_then(codex_entry_root) else {
        return;
    };

    let target_name = unique_codex_server_name_ignoring(servers, &legacy_root, Some("repolens"));
    let Some(legacy_item) = servers.remove("repolens") else {
        return;
    };

    if let Some(existing_root) = servers.get(&target_name).and_then(codex_entry_root)
        && same_project_root(&existing_root, &legacy_root)
    {
        return;
    }

    servers.insert(&target_name, legacy_item);
}

fn find_codex_server_for_root(servers: &Table, root: &Path) -> Option<String> {
    repolens_codex_entries(servers)
        .into_iter()
        .find_map(|(name, registered_root)| {
            same_project_root(&registered_root, root).then_some(name)
        })
}

fn unique_codex_server_name(servers: &Table, root: &Path) -> String {
    unique_codex_server_name_ignoring(servers, root, None)
}

fn unique_codex_server_name_ignoring(
    servers: &Table,
    root: &Path,
    ignored_name: Option<&str>,
) -> String {
    let base = codex_server_name(root);
    let mut candidate = base.clone();
    let mut suffix = 2usize;

    loop {
        if ignored_name == Some(candidate.as_str()) || !servers.contains_key(&candidate) {
            return candidate;
        }
        if servers
            .get(&candidate)
            .and_then(codex_entry_root)
            .is_some_and(|registered_root| same_project_root(&registered_root, root))
        {
            return candidate;
        }
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
}

fn codex_server_name(root: &Path) -> String {
    let components = root
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter(|component| !component.ends_with(':'))
        .map(slugify)
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let leaf = components.last().map(String::as_str).unwrap_or("project");
    let parent = components
        .get(components.len().saturating_sub(2))
        .map(String::as_str)
        .filter(|parent| *parent != leaf);
    let slug = readable_project_slug(parent, leaf, 44);

    let identity = normalized_root_identity(root);
    let hash = blake3::hash(identity.as_bytes()).to_hex();
    format!("repolens_{slug}_{}", &hash[..8])
}

fn readable_project_slug(parent: Option<&str>, leaf: &str, max_len: usize) -> String {
    let mut leaf = leaf.to_owned();
    leaf.truncate(max_len);
    let Some(parent) = parent else {
        return leaf;
    };
    let parent_budget = max_len.saturating_sub(leaf.len() + 1);
    if parent_budget == 0 {
        return leaf;
    }
    let mut parent = parent.to_owned();
    parent.truncate(parent_budget);
    format!("{parent}_{leaf}")
}

fn slugify(input: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('_');
            separator = true;
        }
    }
    output.trim_matches('_').to_owned()
}

fn normalized_root_identity(root: &Path) -> String {
    let identity = root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        identity.to_ascii_lowercase()
    } else {
        identity
    }
}

fn same_project_root(left: &Path, right: &Path) -> bool {
    normalized_root_identity(left) == normalized_root_identity(right)
}

fn write_codex_server(servers: &mut Table, name: &str, exe: &Path, root: &Path) {
    if !servers.contains_key(name) {
        servers.insert(name, Item::Table(Table::new()));
    }
    let Some(server) = servers.get_mut(name).and_then(Item::as_table_mut) else {
        return;
    };

    server["command"] = value(exe.to_string_lossy().as_ref());
    let mut args = Array::new();
    args.push("mcp");
    args.push(root.to_string_lossy().as_ref());
    server["args"] = value(args);
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

fn repolens_codex_entries(servers: &Table) -> Vec<(String, PathBuf)> {
    servers
        .iter()
        .filter_map(|(name, item)| codex_entry_root(item).map(|root| (name.to_owned(), root)))
        .collect()
}

fn write_config_if_changed(path: &Path, existing: &str, next: String) -> Result<()> {
    if existing == next {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    backup_file(path)?;
    fs::write(path, next).with_context(|| format!("failed to write {}", path.display()))
}

fn write_json_mcp_config(path: &Path, exe: &Path, root: &Path) -> Result<()> {
    let mut config = if path.exists() {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !config.is_object() {
        config = json!({});
    }
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = json!({});
    }
    config["mcpServers"]["repolens"] = json!({
        "command": exe.to_string_lossy(),
        "args": ["mcp", root.to_string_lossy()]
    });

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    backup_file(path)?;
    fs::write(path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn remove_json_mcp_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut config = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}));
    if let Some(servers) = config.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove("repolens");
    }
    backup_file(path)?;
    fs::write(path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn backup_file(path: &Path) -> Result<()> {
    if path.exists() {
        let backup = path.with_extension(format!(
            "{}bak",
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| format!("{ext}."))
                .unwrap_or_default()
        ));
        fs::copy(path, &backup).with_context(|| {
            format!(
                "failed to write backup {} from {}",
                backup.display(),
                path.display()
            )
        })?;
    }
    Ok(())
}

fn codex_project_count(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let document = raw
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(servers) = document.get("mcp_servers").and_then(Item::as_table) else {
        return Ok(0);
    };
    Ok(repolens_codex_entries(servers).len())
}

fn json_mcp_enabled(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let config = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}));
    Ok(config
        .get("mcpServers")
        .and_then(Value::as_object)
        .is_some_and(|servers| servers.contains_key("repolens")))
}

fn enabled_label(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

fn codex_status_label(project_count: usize) -> String {
    match project_count {
        0 => "disabled".to_owned(),
        1 => "enabled (1 project)".to_owned(),
        count => format!("enabled ({count} projects)"),
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;
    use toml_edit::{DocumentMut, Item};

    use super::{
        codex_project_count, codex_server_name, json_mcp_enabled, remove_codex_config,
        remove_json_mcp_config, repolens_codex_entries, write_codex_config,
    };

    #[test]
    fn removes_json_mcp_server_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let raw = serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "repolens": {"command": "repolens", "args": ["mcp", "."]},
                "other": {"command": "other"}
            }
        }))
        .unwrap();
        std::fs::write(&path, raw).unwrap();

        assert!(json_mcp_enabled(&path).unwrap());
        remove_json_mcp_config(&path).unwrap();
        assert!(!json_mcp_enabled(&path).unwrap());

        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(updated["mcpServers"].get("other").is_some());
    }

    #[test]
    fn codex_server_names_are_stable_and_path_specific() {
        let first = Path::new("C:/work/client-a/frontend");
        let second = Path::new("C:/work/client-b/frontend");
        assert_eq!(codex_server_name(first), codex_server_name(first));
        assert_ne!(codex_server_name(first), codex_server_name(second));
        assert!(codex_server_name(first).starts_with("repolens_client_a_frontend_"));
    }

    #[test]
    fn codex_server_name_keeps_leaf_when_parent_is_long() {
        let root = Path::new(
            "C:/this-is-an-extremely-long-parent-directory-name-that-must-be-truncated/project-a",
        );
        let name = codex_server_name(root);
        assert!(name.contains("project_a_"));
        assert!(name.len() <= 62);
    }

    #[test]
    fn codex_config_preserves_multiple_projects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let first = dir.path().join("project-a");
        let second = dir.path().join("project-b");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let exe = Path::new("C:/bin/repolens.exe");

        write_codex_config(&path, exe, &first).unwrap();
        write_codex_config(&path, exe, &second).unwrap();
        write_codex_config(&path, exe, &first).unwrap();

        assert_eq!(codex_project_count(&path).unwrap(), 2);
        let document = parse_document(&path);
        let servers = document["mcp_servers"].as_table().unwrap();
        assert!(!servers.contains_key("repolens"));
        assert_eq!(repolens_codex_entries(servers).len(), 2);
    }

    #[test]
    fn codex_config_migrates_legacy_entry_without_losing_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let legacy_root = dir.path().join("legacy-project");
        let new_root = dir.path().join("new-project");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::create_dir_all(&new_root).unwrap();
        let legacy_root = legacy_root.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &path,
            format!(
                "# keep this comment\n[mcp_servers.other]\ncommand = \"other\"\n\n[mcp_servers.repolens]\ncommand = \"C:\\\\bin\\\\repolens.exe\"\nargs = [\"mcp\", \"{legacy_root}\"]\n"
            ),
        )
        .unwrap();

        write_codex_config(&path, Path::new("C:/bin/repolens.exe"), &new_root).unwrap();

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("# keep this comment"));
        let document = updated.parse::<DocumentMut>().unwrap();
        let servers = document["mcp_servers"].as_table().unwrap();
        assert!(servers.contains_key("other"));
        assert!(!servers.contains_key("repolens"));
        assert_eq!(repolens_codex_entries(servers).len(), 2);
    }

    #[test]
    fn disabling_codex_removes_all_repolens_projects_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let first = dir.path().join("project-a");
        let second = dir.path().join("project-b");
        let exe = Path::new("C:/bin/repolens.exe");

        write_codex_config(&path, exe, &first).unwrap();
        write_codex_config(&path, exe, &second).unwrap();
        let mut document = parse_document(&path);
        document["mcp_servers"]["other"] = Item::Table(toml_edit::Table::new());
        document["mcp_servers"]["other"]["command"] = toml_edit::value("other");
        std::fs::write(&path, document.to_string()).unwrap();

        remove_codex_config(&path).unwrap();

        assert_eq!(codex_project_count(&path).unwrap(), 0);
        let document = parse_document(&path);
        assert!(document["mcp_servers"]["other"].is_table());
    }

    #[test]
    fn invalid_codex_toml_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let invalid = "[mcp_servers.repolens\ninvalid";
        std::fs::write(&path, invalid).unwrap();

        assert!(write_codex_config(&path, Path::new("repolens.exe"), dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    }

    fn parse_document(path: &Path) -> DocumentMut {
        std::fs::read_to_string(path)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap()
    }
}
