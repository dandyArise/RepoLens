use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::pathing::canonical_utf8;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum GainFormat {
    Text,
    Json,
}

#[derive(Debug, Serialize)]
pub struct UsageEvent {
    v: u8,
    ts: String,
    session: String,
    session_source: String,
    cmd: String,
    mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parser: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback: Option<bool>,
    bytes_raw: usize,
    bytes_out: usize,
    tokens_raw_est: usize,
    tokens_out_est: usize,
    tokens_saved_est: usize,
}

#[derive(Debug)]
pub struct UsageInput<'a> {
    pub cmd: &'a str,
    pub file: Option<&'a Utf8Path>,
    pub level: Option<&'a str>,
    pub parser: Option<&'a str>,
    pub fallback: Option<bool>,
    pub bytes_raw: usize,
    pub bytes_out: usize,
}

#[derive(Debug, Deserialize)]
struct UsageRecord {
    #[serde(default)]
    mode: String,
    level: Option<String>,
    parser: Option<String>,
    bytes_raw: usize,
    bytes_out: usize,
    tokens_raw_est: usize,
    tokens_out_est: usize,
    tokens_saved_est: usize,
}

#[derive(Debug, Serialize, Default, Clone)]
struct GainBucket {
    events: usize,
    bytes_raw: usize,
    bytes_out: usize,
    tokens_raw_est: usize,
    tokens_out_est: usize,
    tokens_saved_est: usize,
    reduction_ratio: f64,
}

#[derive(Debug, Serialize)]
struct GainReport {
    period: String,
    generated_at: String,
    events: usize,
    by_mode: BTreeMap<String, GainBucket>,
    by_level: BTreeMap<String, GainBucket>,
    by_parser: BTreeMap<String, GainBucket>,
    total: GainBucket,
}

pub fn log_usage(root: &Utf8Path, input: UsageInput<'_>) {
    let _ = try_log_usage(root, input);
}

pub fn print_gain(root: &std::path::Path, format: GainFormat) -> Result<()> {
    let root = canonical_utf8(root)?;
    let path = usage_path(&root);
    if !path.exists() {
        println!("No usage data found. Run some commands first.");
        return Ok(());
    }

    let file = fs::File::open(&path).with_context(|| format!("failed to read {path}"))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<UsageRecord>(&line) {
            records.push(record);
        }
    }

    if records.is_empty() {
        println!("No usage data found. Run some commands first.");
        return Ok(());
    }

    let report = build_gain_report(&records);
    match format {
        GainFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        GainFormat::Text => print_gain_text(&report),
    }
    Ok(())
}

fn try_log_usage(root: &Utf8Path, input: UsageInput<'_>) -> Result<()> {
    let dir = root.join(".repolens");
    fs::create_dir_all(&dir)?;
    let (session, session_source) = resolve_session(&dir);
    let bytes_raw = input.bytes_raw;
    let bytes_out = input.bytes_out;
    let tokens_raw_est = estimate_tokens(bytes_raw);
    let tokens_out_est = estimate_tokens(bytes_out);
    let event = UsageEvent {
        v: 1,
        ts: utc_now(),
        session,
        session_source,
        cmd: input.cmd.to_string(),
        mode: "cli".to_string(),
        file: input.file.map(|path| path.as_str().replace('\\', "/")),
        level: input.level.map(str::to_string),
        parser: input.parser.map(str::to_string),
        fallback: input.fallback,
        bytes_raw,
        bytes_out,
        tokens_raw_est,
        tokens_out_est,
        tokens_saved_est: tokens_raw_est.saturating_sub(tokens_out_est),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(usage_path(root))?;
    writeln!(file, "{}", serde_json::to_string(&event)?)?;
    Ok(())
}

fn usage_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".repolens").join("usage.jsonl")
}

fn resolve_session(dir: &Utf8Path) -> (String, String) {
    if let Ok(value) = std::env::var("REPOLENS_SESSION")
        && !value.trim().is_empty()
    {
        return (value, "env".to_string());
    }

    let path = dir.join("session");
    if let Ok(value) = fs::read_to_string(&path) {
        let value = value.trim();
        if !value.is_empty() {
            return (value.to_string(), "repo".to_string());
        }
    }

    let id = generate_session_id();
    if fs::write(&path, &id).is_ok() {
        return (id, "repo".to_string());
    }

    (generate_session_id(), "ephemeral".to_string())
}

fn build_gain_report(records: &[UsageRecord]) -> GainReport {
    let mut by_mode = BTreeMap::new();
    let mut by_level = BTreeMap::new();
    let mut by_parser = BTreeMap::new();
    let mut total = GainBucket::default();

    for record in records {
        add_record(&mut total, record);
        add_record(
            by_mode
                .entry(non_empty(&record.mode, "unknown"))
                .or_default(),
            record,
        );
        if let Some(level) = &record.level {
            add_record(by_level.entry(level.clone()).or_default(), record);
        }
        let parser = record
            .parser
            .clone()
            .unwrap_or_else(|| "fallback".to_string());
        add_record(by_parser.entry(parser).or_default(), record);
    }

    finalize_bucket(&mut total);
    for bucket in by_mode.values_mut() {
        finalize_bucket(bucket);
    }
    for bucket in by_level.values_mut() {
        finalize_bucket(bucket);
    }
    for bucket in by_parser.values_mut() {
        finalize_bucket(bucket);
    }

    GainReport {
        period: "all".to_string(),
        generated_at: utc_now(),
        events: records.len(),
        by_mode,
        by_level,
        by_parser,
        total,
    }
}

fn add_record(bucket: &mut GainBucket, record: &UsageRecord) {
    bucket.events += 1;
    bucket.bytes_raw += record.bytes_raw;
    bucket.bytes_out += record.bytes_out;
    bucket.tokens_raw_est += record.tokens_raw_est;
    bucket.tokens_out_est += record.tokens_out_est;
    bucket.tokens_saved_est += record.tokens_saved_est;
}

fn finalize_bucket(bucket: &mut GainBucket) {
    bucket.reduction_ratio = if bucket.tokens_raw_est == 0 {
        0.0
    } else {
        bucket.tokens_saved_est as f64 / bucket.tokens_raw_est as f64
    };
}

fn print_gain_text(report: &GainReport) {
    println!("RepoLens gain - all time");
    println!("Events logged    : {}", report.events);
    println!("Tokens raw est.  : {}", report.total.tokens_raw_est);
    println!("Tokens out est.  : {}", report.total.tokens_out_est);
    println!("Tokens saved     : {}", report.total.tokens_saved_est);
    println!(
        "Reduction        : {:.1}%",
        report.total.reduction_ratio * 100.0
    );
    println!();
    println!("By mode:");
    for (mode, bucket) in &report.by_mode {
        println!(
            "  {mode} ({}) : {:.1}% reduction",
            bucket.events,
            bucket.reduction_ratio * 100.0
        );
    }
    println!();
    println!("By level:");
    for (level, bucket) in &report.by_level {
        println!(
            "  {level} ({}) : {:.1}% reduction",
            bucket.events,
            bucket.reduction_ratio * 100.0
        );
    }
    println!();
    println!("By parser:");
    for (parser, bucket) in &report.by_parser {
        println!("  {parser} ({})", bucket.events);
    }
}

fn estimate_tokens(bytes: usize) -> usize {
    // Approximation: token values are estimates, not exact tokenizer counts.
    bytes / 4
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn generate_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("sess_{nanos:x}")
}

fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    format_unix_utc(seconds)
}

fn format_unix_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{UsageRecord, build_gain_report, civil_from_days, format_unix_utc};

    #[test]
    fn formats_unix_epoch_as_utc() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn converts_known_day() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn gain_report_groups_usage_records() {
        let records = vec![
            UsageRecord {
                mode: "cli".to_string(),
                level: Some("compact".to_string()),
                parser: Some("tree-sitter-rust".to_string()),
                bytes_raw: 400,
                bytes_out: 100,
                tokens_raw_est: 100,
                tokens_out_est: 25,
                tokens_saved_est: 75,
            },
            UsageRecord {
                mode: "cli".to_string(),
                level: Some("aggressive".to_string()),
                parser: None,
                bytes_raw: 800,
                bytes_out: 200,
                tokens_raw_est: 200,
                tokens_out_est: 50,
                tokens_saved_est: 150,
            },
        ];

        let report = build_gain_report(&records);

        assert_eq!(report.events, 2);
        assert_eq!(report.total.tokens_saved_est, 225);
        assert_eq!(report.by_mode["cli"].events, 2);
        assert_eq!(report.by_level["compact"].tokens_saved_est, 75);
        assert_eq!(report.by_parser["fallback"].events, 1);
        assert!((report.total.reduction_ratio - 0.75).abs() < f64::EPSILON);
    }
}
