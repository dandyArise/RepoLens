use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub max_file_size: u64,
    pub allow_sensitive: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    max_file_size: Option<String>,
    allow_sensitive: Option<bool>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".repolensrc.toml");
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let raw: RawConfig = toml::from_str(&raw).context("invalid .repolensrc.toml")?;
        Ok(Self {
            max_file_size: raw
                .max_file_size
                .as_deref()
                .map(parse_size)
                .transpose()?
                .unwrap_or(Self::default().max_file_size),
            allow_sensitive: raw.allow_sensitive.unwrap_or(false),
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024,
            allow_sensitive: false,
        }
    }
}

fn parse_size(raw: &str) -> Result<u64> {
    let raw = raw.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(number) = raw.strip_suffix("kb") {
        (number.trim(), 1024)
    } else if let Some(number) = raw.strip_suffix("mb") {
        (number.trim(), 1024 * 1024)
    } else if let Some(number) = raw.strip_suffix('b') {
        (number.trim(), 1)
    } else {
        (raw.as_str(), 1)
    };

    let number = number.parse::<u64>().context("invalid size")?;
    Ok(number * multiplier)
}

#[cfg(test)]
mod tests {
    use super::{Config, parse_size};

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("10").unwrap(), 10);
        assert_eq!(parse_size("2kb").unwrap(), 2048);
        assert_eq!(parse_size("3mb").unwrap(), 3 * 1024 * 1024);
    }

    #[test]
    fn defaults_are_safe() {
        let config = Config::default();
        assert_eq!(config.max_file_size, 1024 * 1024);
        assert!(!config.allow_sensitive);
    }
}
