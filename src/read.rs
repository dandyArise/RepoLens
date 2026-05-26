use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::pathing::{canonical_utf8, safe_join};

pub fn read(root: &Path, path: &Utf8Path, lines: Option<&str>) -> Result<()> {
    let root = canonical_utf8(root)?;
    let target = safe_join(&root, path)?;
    let content =
        fs::read_to_string(&target).with_context(|| format!("failed to read {target}"))?;
    print_lines(&content, lines)
}

fn print_lines(content: &str, range: Option<&str>) -> Result<()> {
    let (start, end) = match range {
        Some(raw) => parse_line_range(raw)?,
        None => (1, usize::MAX),
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no >= start && line_no <= end {
            writeln!(out, "{line_no}: {line}")?;
        }
        if line_no > end {
            break;
        }
    }
    Ok(())
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
