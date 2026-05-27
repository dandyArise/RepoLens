use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;

use crate::index::ProjectIndex;
use crate::{search, snapshot, symbols};

#[derive(Debug, Serialize)]
struct BenchReport {
    index_ms: u128,
    save_ms: u128,
    load_ms: u128,
    files: usize,
    words: usize,
    trigrams: usize,
    symbols: usize,
    deps_files: usize,
    snapshot_json_bytes: u64,
    snapshot_bin_bytes: u64,
    search_ms: u128,
    search_hits: usize,
    symbol_ms: u128,
    symbol_hits: usize,
    rg_ms: Option<u128>,
}

pub fn run(root: &Path, query: &str, symbol: &str, limit: usize, json: bool) -> Result<()> {
    let index_start = Instant::now();
    let index = ProjectIndex::build(root)?;
    let index_time = index_start.elapsed();

    let save_start = Instant::now();
    snapshot::save(&index)?;
    let save_time = save_start.elapsed();

    let load_start = Instant::now();
    let loaded = snapshot::load_or_build(root)?;
    let load_time = load_start.elapsed();
    let info = snapshot::info(&loaded);

    let search_start = Instant::now();
    let hits = search::search_hits(&loaded, query, limit)?;
    let search_time = search_start.elapsed();

    let symbol_start = Instant::now();
    let symbols = symbols::find(&loaded, symbol, limit);
    let symbol_time = symbol_start.elapsed();

    let report = BenchReport {
        index_ms: ms(index_time),
        save_ms: ms(save_time),
        load_ms: ms(load_time),
        files: loaded.files.len(),
        words: loaded.words.len(),
        trigrams: loaded.trigrams.len(),
        symbols: loaded.symbols.len(),
        deps_files: loaded.deps.len(),
        snapshot_json_bytes: info.bytes,
        snapshot_bin_bytes: info.binary_bytes,
        search_ms: ms(search_time),
        search_hits: hits.len(),
        symbol_ms: ms(symbol_time),
        symbol_hits: symbols.len(),
        rg_ms: run_rg(root, query, limit).map(ms),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }

    Ok(())
}

fn print_report(report: &BenchReport) {
    println!("index_ms: {}", report.index_ms);
    println!("save_ms: {}", report.save_ms);
    println!("load_ms: {}", report.load_ms);
    println!("files: {}", report.files);
    println!("words: {}", report.words);
    println!("trigrams: {}", report.trigrams);
    println!("symbols: {}", report.symbols);
    println!("deps_files: {}", report.deps_files);
    println!("snapshot_json_bytes: {}", report.snapshot_json_bytes);
    println!("snapshot_bin_bytes: {}", report.snapshot_bin_bytes);
    println!("search_ms: {}", report.search_ms);
    println!("search_hits: {}", report.search_hits);
    println!("symbol_ms: {}", report.symbol_ms);
    println!("symbol_hits: {}", report.symbol_hits);
    match report.rg_ms {
        Some(ms) => println!("rg_ms: {ms}"),
        None => println!("rg_ms: unavailable"),
    }
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
