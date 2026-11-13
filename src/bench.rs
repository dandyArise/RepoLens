use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::index::ProjectIndex;
use crate::{search, snapshot, symbols};

#[derive(Debug, Serialize)]
struct BenchReport {
    index_ms: f64,
    save_ms: f64,
    load_ms: f64,
    files: usize,
    words: usize,
    trigrams: usize,
    symbols: usize,
    deps_files: usize,
    snapshot_json_bytes: u64,
    snapshot_bin_bytes: u64,
    search_ms: f64,
    search_hits: usize,
    symbol_ms: f64,
    symbol_hits: usize,
    cli_comparison: CliComparison,
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
        cli_comparison: compare_cli(&loaded, query),
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
    let comparison = &report.cli_comparison;
    println!(
        "\nThree-mode comparison: {:?} ({})",
        comparison.query, comparison.build_profile
    );
    println!("{}", comparison.protocol);
    println!("{}", comparison.rg_selection);
    if !comparison.comparable {
        println!(
            "INVALID: {}",
            comparison.reason.as_deref().unwrap_or("unknown failure")
        );
        return;
    }
    println!(
        "Equality: verified on all {} rounds; {} matching lines in {} files",
        comparison.verified_rounds, comparison.hits, comparison.files
    );
    println!(
        "MCP startup + initialize: {:.3} ms (separate)",
        comparison.mcp_startup_ms.unwrap()
    );
    println!(
        "{:<20} {:>12} {:>12} {:>12} {:>12}",
        "Mode", "Median ms", "Mean ms", "Min ms", "Max ms"
    );
    for (name, stats) in [
        ("RepoLens CLI", &comparison.repolens_cli_summary),
        ("RepoLens MCP", &comparison.repolens_mcp_summary),
        ("rg CLI", &comparison.rg_cli_summary),
    ] {
        let stats = stats.as_ref().unwrap();
        println!(
            "{name:<20} {:>12.3} {:>12.3} {:>12.3} {:>12.3}",
            stats.median_ms, stats.mean_ms, stats.min_ms, stats.max_ms
        );
    }
}

#[derive(Debug, Serialize, Default)]
struct CliComparison {
    protocol: &'static str,
    rg_selection: &'static str,
    query: String,
    build_profile: &'static str,
    comparable: bool,
    reason: Option<String>,
    files: usize,
    hits: usize,
    repolens_cli_ms: Vec<f64>,
    repolens_mcp_ms: Vec<f64>,
    rg_cli_ms: Vec<f64>,
    mcp_startup_ms: Option<f64>,
    mcp_pid: Option<u32>,
    verified_rounds: usize,
    repolens_cli_summary: Option<TimingSummary>,
    repolens_mcp_summary: Option<TimingSummary>,
    rg_cli_summary: Option<TimingSummary>,
}

#[derive(Debug, Serialize)]
struct TimingSummary {
    median_ms: f64,
    mean_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

fn summarize(samples: &[f64]) -> TimingSummary {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    TimingSummary {
        median_ms: sorted[sorted.len() / 2],
        mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        min_ms: sorted[0],
        max_ms: sorted[sorted.len() - 1],
    }
}

fn compare_cli(index: &ProjectIndex, query: &str) -> CliComparison {
    let mut report = CliComparison {
        protocol: "warm cache; all hits; indexed files only; literal case-insensitive; one warmup then 7 rounds with rotating mode order; CLI startup/output capture and MCP request/response transport included; client parsing/validation excluded; native outputs: CLI text, MCP JSON; MCP startup separate; corpus and snapshots checked before/after",
        query: query.to_owned(),
        rg_selection: "rg: temporary whitelist file, exact indexed file set verified before/after; one process per sample; directory traversal and whitelist parsing included; whitelist creation and file-set verification excluded",
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        files: index.files.len(),
        ..Default::default()
    };
    if let Err(error) = measure_cli(index, query, &mut report) {
        report.reason = Some(format!("{error:#}"));
        report.repolens_cli_ms.clear();
        report.repolens_mcp_ms.clear();
        report.rg_cli_ms.clear();
    } else {
        report.comparable = true;
        report.repolens_cli_summary = Some(summarize(&report.repolens_cli_ms));
        report.repolens_mcp_summary = Some(summarize(&report.repolens_mcp_ms));
        report.rg_cli_summary = Some(summarize(&report.rg_cli_ms));
    }
    report
}

fn measure_cli(index: &ProjectIndex, query: &str, report: &mut CliComparison) -> Result<()> {
    if index.files.is_empty() {
        bail!("empty corpus: no CLI comparison");
    }
    if query.contains(['\n', '\r']) {
        bail!("multiline queries have different semantics");
    }
    let mut repolens = Command::new(std::env::current_exe()?);
    repolens.current_dir(index.root.as_std_path()).args([
        "search",
        "--limit",
        &usize::MAX.to_string(),
        "--",
        query,
        ".",
    ]);
    let corpus = RgCorpus::new(index)?;
    corpus.verify(index)?;
    let mut rg = corpus.command(index);
    rg.args([
        "--fixed-strings",
        "--ignore-case",
        "--text",
        "--encoding",
        "none",
        "--line-number",
        "--with-filename",
        "--no-heading",
        "--color=never",
        "--",
        query,
        ".",
    ]);
    let before = corpus_fingerprint(index)?;
    let snapshots = snapshot_fingerprint(index)?;
    let (mut mcp, startup) = McpClient::start(index.root.as_std_path())?;
    report.mcp_startup_ms = Some(ms(startup));
    report.mcp_pid = Some(mcp.child.id());
    for round in 0..8 {
        let mut results = Vec::new();
        for offset in 0..3 {
            let mode = (round + offset) % 3;
            let result = match mode {
                0 => capture(&mut repolens, false)?,
                1 => mcp.search(query)?,
                _ => capture(&mut rg, true)?,
            };
            results.push((mode, result));
        }
        results.sort_by_key(|(mode, _)| *mode);
        let left = &results[0].1;
        let resident = &results[1].1;
        let right = &results[2].1;
        let mut actual: Vec<String> = std::str::from_utf8(&left.1)?
            .lines()
            .map(|line| {
                let (location, text) = line.split_once(": ").unwrap_or((line, ""));
                format!("{}: {}", location.replace('\\', "/"), text)
            })
            .collect();
        actual.sort();
        let expected = rg_hits(&right.1)?;
        let resident_hits = mcp_hits(&resident.1)?;
        verify_hits(&actual, &resident_hits, &expected)?;
        report.verified_rounds += 1;
        report.hits = actual.len();
        if round > 0 {
            report.repolens_cli_ms.push(left.0.as_secs_f64() * 1000.0);
            report.repolens_mcp_ms.push(ms(resident.0));
            report.rg_cli_ms.push(right.0.as_secs_f64() * 1000.0);
        }
    }
    if before != corpus_fingerprint(index)? || snapshots != snapshot_fingerprint(index)? {
        bail!("corpus or index snapshot changed during benchmark");
    }
    corpus.verify(index)?;
    Ok(())
}

// A temporary gitignore-format whitelist keeps the command length independent
// of the file count. rg still runs once per sample; traversal and rule parsing
// are part of its measured cost. Verify the actual file set outside the timer.
struct RgCorpus {
    rules: tempfile::NamedTempFile,
}

impl RgCorpus {
    fn new(index: &ProjectIndex) -> Result<Self> {
        let mut directories = BTreeSet::new();
        let mut files = Vec::new();
        for file in &index.files {
            let path = file.path.as_str().replace('\\', "/");
            if path.contains([':', '\n', '\r']) {
                bail!("file path cannot be represented unambiguously in CLI text output");
            }
            for (offset, _) in path.match_indices('/') {
                directories.insert(escape_ignore_path(&path[..offset]));
            }
            files.push(escape_ignore_path(&path));
        }
        let mut rules = tempfile::NamedTempFile::new()?;
        writeln!(rules, "*")?;
        for directory in directories {
            writeln!(rules, "!/{directory}/")?;
        }
        for file in files {
            writeln!(rules, "!/{file}")?;
        }
        rules.flush()?;
        Ok(Self { rules })
    }

    fn command(&self, index: &ProjectIndex) -> Command {
        let mut command = Command::new("rg");
        command
            .current_dir(index.root.as_std_path())
            .args(["--no-config", "--no-ignore", "--hidden", "--ignore-file"])
            .arg(self.rules.path());
        command
    }

    fn verify(&self, index: &ProjectIndex) -> Result<()> {
        let output = self
            .command(index)
            .args(["--files", "--null", "."])
            .output()
            .context("cannot launch rg corpus verification")?;
        if !output.status.success() || !output.stderr.is_empty() {
            bail!(
                "rg corpus verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let mut actual: Vec<_> = std::str::from_utf8(&output.stdout)?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| {
                let path = path.replace('\\', "/");
                path.strip_prefix("./").unwrap_or(&path).to_owned()
            })
            .collect();
        let mut expected: Vec<_> = index
            .files
            .iter()
            .map(|file| file.path.as_str().replace('\\', "/"))
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            bail!(
                "rg corpus mismatch: expected {} indexed files, selected {}",
                expected.len(),
                actual.len()
            );
        }
        Ok(())
    }
}

fn escape_ignore_path(path: &str) -> String {
    let mut escaped = String::new();
    for ch in path.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\' | '!' | '#' | ' ') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn verify_hits(cli: &[String], mcp: &[String], rg: &[String]) -> Result<()> {
    if cli != rg || mcp != rg {
        bail!(
            "result mismatch: CLI {} lines, MCP {} lines, rg {} lines; paths, line numbers and text must agree (Unicode case-folding may differ)",
            cli.len(),
            mcp.len(),
            rg.len()
        );
    }
    Ok(())
}

fn corpus_fingerprint(index: &ProjectIndex) -> Result<Vec<blake3::Hash>> {
    index
        .files
        .iter()
        .map(|file| Ok(blake3::hash(&std::fs::read(index.root.join(&file.path))?)))
        .collect()
}

fn snapshot_fingerprint(index: &ProjectIndex) -> Result<Vec<blake3::Hash>> {
    ["index.json", "index.bin"]
        .iter()
        .map(|name| {
            Ok(blake3::hash(&std::fs::read(
                index.root.join(".repolens").join(name),
            )?))
        })
        .collect()
}

// Own the child so every success/error path terminates and reaps the server.
struct McpClient {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<std::io::Result<Vec<u8>>>,
    next_id: u64,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl McpClient {
    fn start(root: &Path) -> Result<(Self, Duration)> {
        let start = Instant::now();
        let mut child = Command::new(std::env::current_exe()?)
            .arg("mcp")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("cannot start persistent MCP benchmark server")?;
        let stdin = child.stdin.take().context("missing MCP stdin")?;
        let stdout = child.stdout.take().context("missing MCP stdout")?;
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            responses,
            next_id: 0,
        };
        let (_, response) = client.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "repolens-bench", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        rpc_result(&response)?;
        client
            .stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")?;
        client.stdin.flush()?;
        Ok((client, start.elapsed()))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<(Duration, Vec<u8>)> {
        self.next_id += 1;
        let mut request = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}),
        )?;
        request.push(b'\n');
        let start = Instant::now();
        self.stdin.write_all(&request)?;
        self.stdin.flush()?;
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(30))
            .context("MCP response timed out or server closed stdout")??;
        let elapsed = start.elapsed();
        let parsed: Value = serde_json::from_slice(&response)?;
        if parsed["id"].as_u64() != Some(self.next_id) {
            bail!("MCP response ID mismatch");
        }
        Ok((elapsed, response))
    }

    fn search(&mut self, query: &str) -> Result<(Duration, Vec<u8>)> {
        self.request(
            "tools/call",
            json!({"name": "repolens_search", "arguments": {"query": query, "limit": usize::MAX}}),
        )
    }
}

fn rpc_result(raw: &[u8]) -> Result<Value> {
    let response: Value = serde_json::from_slice(raw)?;
    if let Some(error) = response.get("error") {
        bail!("MCP RPC error: {error}");
    }
    let result = response.get("result").context("missing MCP result")?;
    if result["isError"].as_bool() == Some(true) {
        bail!("MCP tool error: {result}");
    }
    Ok(result.clone())
}

fn mcp_hits(raw: &[u8]) -> Result<Vec<String>> {
    let result = rpc_result(raw)?;
    let text = result["content"][0]["text"]
        .as_str()
        .context("missing MCP search content")?;
    let entries: Vec<Value> = serde_json::from_str(text)?;
    let mut hits = Vec::new();
    for entry in entries {
        let path = entry["path"].as_str().context("missing MCP hit path")?;
        let line = entry["line"].as_u64().context("missing MCP hit line")?;
        let text = entry["text"].as_str().context("missing MCP hit text")?;
        hits.push(format!("{}:{}: {}", path.replace('\\', "/"), line, text));
    }
    hits.sort();
    Ok(hits)
}

fn capture(command: &mut Command, no_match_ok: bool) -> Result<(Duration, Vec<u8>)> {
    let start = Instant::now();
    let output = command.output().context(
        "cannot launch benchmark command (missing executable or argument list too long)",
    )?;
    let elapsed = start.elapsed();
    if !output.status.success() && !(no_match_ok && output.status.code() == Some(1)) {
        bail!(
            "benchmark command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok((elapsed, output.stdout))
}

fn rg_hits(output: &[u8]) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    for line in std::str::from_utf8(output)?.lines() {
        let mut fields = line.splitn(3, ':');
        let path = fields.next().context("missing rg path")?;
        let number: usize = fields.next().context("missing rg line number")?.parse()?;
        let text = fields.next().context("missing rg match text")?;
        let path = path.replace('\\', "/");
        hits.push(format!(
            "{}:{}: {}",
            path.strip_prefix("./").unwrap_or(&path),
            number,
            text.trim()
        ));
    }
    hits.sort();
    Ok(hits)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rg_whitelist_selects_exact_paths_with_glob_characters() {
        if Command::new("rg").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder [1] #!");
        std::fs::create_dir(&folder).unwrap();
        for name in ["file [x].rs", "-dash.rs", "é space.rs"] {
            std::fs::write(folder.join(name), "needle\n").unwrap();
        }
        let mut index = ProjectIndex::build(dir.path()).unwrap();
        assert_eq!(index.files.len(), 3);
        // Exercise hidden paths without relying on the scanner's hidden policy.
        std::fs::write(dir.path().join(".hidden.rs"), "needle\n").unwrap();
        index.files[0].path = ".hidden.rs".into();
        std::fs::write(dir.path().join("unindexed.rs"), "needle\n").unwrap();
        std::fs::write(folder.join("file x.rs"), "needle\n").unwrap();
        std::fs::write(dir.path().join(".ignore"), "*\n").unwrap();
        let corpus = RgCorpus::new(&index).unwrap();
        corpus.verify(&index).unwrap();
        let rule_path = corpus.rules.path().to_owned();
        drop(corpus);
        assert!(!rule_path.exists());
    }

    #[test]
    fn equality_checks_content_not_just_counts() {
        let hits = vec!["a.rs:1: needle".to_owned()];
        let different = vec!["a.rs:2: needle".to_owned()];
        assert!(verify_hits(&hits, &hits, &hits).is_ok());
        assert!(verify_hits(&hits, &different, &hits).is_err());
        assert!(verify_hits(&different, &hits, &hits).is_err());
    }

    #[test]
    fn mcp_errors_and_malformed_hits_are_not_empty_successes() {
        assert!(mcp_hits(br#"{"error":{"message":"failure"}}"#).is_err());
        assert!(mcp_hits(br#"{"result":{"isError":true}}"#).is_err());
        assert!(mcp_hits(br#"{"result":{"content":[{"text":"[{}]"}]}}"#).is_err());
        assert!(
            mcp_hits(br#"{"result":{"content":[{"text":"[]"}]}}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn normalizes_rg_matches_without_changing_content() {
        let output = "./b.rs:2:  A\\B\r\n./a.rs:1:a\n";
        assert_eq!(
            rg_hits(output.as_bytes()).unwrap(),
            ["a.rs:1: a", "b.rs:2: A\\B"]
        );
        assert!(rg_hits(b"").unwrap().is_empty());
        assert!(rg_hits(b"invalid output").is_err());
    }
}
