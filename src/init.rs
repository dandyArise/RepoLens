use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cli::InitTargetArg;
use crate::pathing::canonical_utf8;

pub fn run(target: InitTargetArg, root: &Path) -> Result<()> {
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
    fs::write(path, format!("{}{}", next.trim_end(), block))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
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
    use super::remove_toml_table;

    #[test]
    fn removes_existing_codex_table() {
        let input =
            "[other]\na = 1\n[mcp_servers.repolens]\ncommand = \"old\"\nargs = []\n[next]\nb = 2\n";
        let out = remove_toml_table(input, "[mcp_servers.repolens]");
        assert!(out.contains("[other]"));
        assert!(out.contains("[next]"));
        assert!(!out.contains("command = \"old\""));
    }
}
