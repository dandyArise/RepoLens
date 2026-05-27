use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

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
        println!(
            "Codex: {} ({})",
            enabled_label(codex_enabled(&path)?),
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
    let block = format!(
        "\n[mcp_servers.repolens]\ncommand = \"{}\"\nargs = [\"mcp\", \"{}\"]\n",
        toml_escape(exe),
        toml_escape(root)
    );
    merge_text_block(path, "[mcp_servers.repolens]", &block)
}

fn remove_codex_config(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let existing =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let next = remove_toml_table(&existing, "[mcp_servers.repolens]");
    backup_file(path)?;
    fs::write(path, next.trim_end())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
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

fn merge_text_block(path: &Path, marker: &str, block: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let next = remove_toml_table(&existing, marker);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    backup_file(path)?;
    fs::write(path, format!("{}{}", next.trim_end(), block))
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

fn codex_enabled(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(raw
        .lines()
        .any(|line| line.trim() == "[mcp_servers.repolens]"))
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

fn remove_toml_table(input: &str, marker: &str) -> String {
    let mut out = Vec::new();
    let mut skip = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            skip = true;
            continue;
        }
        if skip && trimmed.starts_with('[') {
            skip = false;
        }
        if !skip {
            out.push(line);
        }
    }
    if out.is_empty() {
        String::new()
    } else {
        format!("{}\n", out.join("\n"))
    }
}

fn toml_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
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
    use serde_json::json;

    use super::{json_mcp_enabled, remove_json_mcp_config, remove_toml_table};

    #[test]
    fn removes_existing_codex_table() {
        let input =
            "[other]\na = 1\n[mcp_servers.repolens]\ncommand = \"old\"\nargs = []\n[next]\nb = 2\n";
        let out = remove_toml_table(input, "[mcp_servers.repolens]");
        assert!(out.contains("[other]"));
        assert!(out.contains("[next]"));
        assert!(!out.contains("command = \"old\""));
    }

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
}
