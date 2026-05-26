use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::pathing::{canonical_utf8, safe_join};

pub fn read(
    root: &Path,
    path: &Utf8Path,
    lines: Option<&str>,
    max_bytes: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<()> {
    let content = read_text(root, path, lines, max_bytes, expected_hash)?;
    print!("{content}");
    Ok(())
}

pub fn read_text(
    root: &Path,
    path: &Utf8Path,
    lines: Option<&str>,
    max_bytes: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<String> {
    let root = canonical_utf8(root)?;
    let target = safe_join(&root, path)?;
    let bytes = fs::read(&target).with_context(|| format!("failed to read {target}"))?;
    if let Some(expected_hash) = expected_hash {
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != expected_hash {
            bail!("hash mismatch");
        }
    }
    let bytes = match max_bytes {
        Some(max_bytes) => &bytes[..bytes.len().min(max_bytes)],
        None => &bytes,
    };
    let content = String::from_utf8_lossy(bytes);
    format_lines(&content, lines)
}

fn format_lines(content: &str, range: Option<&str>) -> Result<String> {
    let (start, end) = match range {
        Some(raw) => parse_line_range(raw)?,
        None => (1, usize::MAX),
    };

    let mut out = String::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no >= start && line_no <= end {
            out.push_str(&format!("{line_no}: {line}\n"));
        }
        if line_no > end {
            break;
        }
    }
    Ok(out)
}

fn parse_line_range(raw: &str) -> Result<(usize, usize)> {
    let Some((start, end)) = raw.split_once('-') else {
        let line = raw.parse::<usize>().context("invalid line number")?;
        return Ok((line, line));
    };
    let start = start.parse::<usize>().context("invalid range start")?;
    let end = end.parse::<usize>().context("invalid range end")?;
    if start == 0 || end < start {
        bail!("invalid line range");
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::parse_line_range;

    #[test]
    fn parses_line_ranges() {
        assert_eq!(parse_line_range("2-4").unwrap(), (2, 4));
        assert_eq!(parse_line_range("3").unwrap(), (3, 3));
        assert!(parse_line_range("4-2").is_err());
    }
}
