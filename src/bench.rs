use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::index::ProjectIndex;
use crate::{search, symbols};

pub fn run(root: &Path, query: &str, symbol: &str, limit: usize) -> Result<()> {
    let index_start = Instant::now();
    let index = ProjectIndex::build(root)?;
    let index_time = index_start.elapsed();

    let search_start = Instant::now();
    let hits = search::search_hits(&index, query, limit)?;
    let search_time = search_start.elapsed();

    let symbol_start = Instant::now();
    let symbols = symbols::find(&index, symbol, limit);
    let symbol_time = symbol_start.elapsed();

    println!("index_ms: {}", ms(index_time));
    println!("files: {}", index.files.len());
    println!("words: {}", index.words.len());
    println!("trigrams: {}", index.trigrams.len());
    println!("symbols: {}", index.symbols.len());
    println!("deps_files: {}", index.deps.len());
    println!("search_ms: {}", ms(search_time));
    println!("search_hits: {}", hits.len());
    println!("symbol_ms: {}", ms(symbol_time));
    println!("symbol_hits: {}", symbols.len());

    if let Some(rg_time) = run_rg(root, query, limit) {
        println!("rg_ms: {}", ms(rg_time));
    } else {
        println!("rg_ms: unavailable");
    }

    Ok(())
}

fn run_rg(root: &Path, query: &str, limit: usize) -> Option<Duration> {
    let start = Instant::now();
    let output = Command::new("rg")
        .arg("--fixed-strings")
        .arg("--line-number")
        .arg("--max-count")
        .arg(limit.to_string())
        .arg(query)
        .arg(root)
        .output()
        .ok()?;
    if output.status.success() || output.status.code() == Some(1) {
        Some(start.elapsed())
    } else {
        None
    }
}

fn ms(duration: Duration) -> u128 {
    duration.as_millis()
}
