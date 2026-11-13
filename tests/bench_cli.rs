use std::{fs, process::Command};

#[test]
fn corpus_larger_than_windows_command_limit_works_in_all_modes() {
    if Command::new("rg").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut old_argument_length = 0;
    for i in 0..500 {
        let name = format!("file_{i:04}_{}.rs", "x".repeat(70));
        old_argument_length += name.len() + 3;
        fs::write(dir.path().join(name), "needle\n").unwrap();
    }
    assert!(old_argument_length > 32767);
    let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
        .arg("bench")
        .arg(dir.path())
        .args(["--query=needle", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let c = &report["cli_comparison"];
    assert_eq!(c["comparable"], true, "{c}");
    assert_eq!(c["files"], 500);
    assert_eq!(c["hits"], 500);
    assert_eq!(c["verified_rounds"], 8);
    for mode in ["repolens_cli", "repolens_mcp", "rg_cli"] {
        assert_eq!(c[format!("{mode}_ms")].as_array().unwrap().len(), 7);
    }
}

#[test]
fn bench_compares_complete_results_on_the_indexed_corpus() {
    if Command::new("rg").arg("--version").output().is_err() {
        eprintln!("rg unavailable: CLI comparison integration test skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "Needle\nneedle\n-needle\na.b\n").unwrap();
    fs::write(dir.path().join("b.rs"), "NEEDLE\nneedle\naxb\n").unwrap();
    fs::create_dir(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules/ignored.rs"), "needle\n").unwrap();
    for (query, expected) in [("needle", 5), ("-needle", 1), ("a.b", 1), ("absent", 0)] {
        let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
            .arg("bench")
            .arg(dir.path())
            .arg(format!("--query={query}"))
            .args(["--limit", "1", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let comparison = &report["cli_comparison"];
        assert_eq!(comparison["comparable"], true, "{comparison}");
        assert_eq!(comparison["files"], 2);
        assert_eq!(comparison["hits"], expected);
        assert_eq!(comparison["rg_cli_ms"].as_array().unwrap().len(), 7);
        assert_eq!(comparison["repolens_cli_ms"].as_array().unwrap().len(), 7);
        assert_eq!(comparison["repolens_mcp_ms"].as_array().unwrap().len(), 7);
        assert_eq!(comparison["verified_rounds"], 8);
        assert!(comparison["mcp_startup_ms"].as_f64().unwrap() > 0.0);
        assert!(comparison["mcp_pid"].as_u64().unwrap() > 0);
        for mode in ["repolens_cli", "repolens_mcp", "rg_cli"] {
            let mut samples: Vec<_> = comparison[format!("{mode}_ms")]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            samples.sort_by(f64::total_cmp);
            assert_eq!(
                comparison[format!("{mode}_summary")]["median_ms"],
                samples[3]
            );
        }
        assert_eq!(report["search_hits"], if expected == 0 { 0 } else { 1 });
    }
}

#[test]
fn invalid_comparisons_have_explicit_reasons_and_no_samples() {
    let dir = tempfile::tempdir().unwrap();
    for (query, reason) in [("a", "empty corpus"), ("a\nb", "multiline")] {
        if query.contains('\n') {
            fs::write(dir.path().join("a.rs"), "a\nb\n").unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
            .arg("bench")
            .arg(dir.path())
            .arg(format!("--query={query}"))
            .arg("--json")
            .output()
            .unwrap();
        assert!(output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let comparison = &report["cli_comparison"];
        assert_eq!(comparison["comparable"], false);
        assert!(comparison["reason"].as_str().unwrap().contains(reason));
        assert!(comparison["rg_cli_ms"].as_array().unwrap().is_empty());
        assert!(comparison["repolens_mcp_ms"].as_array().unwrap().is_empty());
        assert!(comparison["repolens_mcp_summary"].is_null());
    }
}

#[test]
fn human_report_labels_all_modes_and_verification() {
    if Command::new("rg").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
        .arg("bench")
        .arg(dir.path())
        .arg("--query=needle")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for label in [
        "RepoLens CLI",
        "RepoLens MCP",
        "rg CLI",
        "Median ms",
        "Equality: verified on all 8 rounds",
        "MCP startup + initialize",
    ] {
        assert!(text.contains(label), "{text}");
    }
}

#[test]
fn unicode_semantic_mismatch_invalidates_all_three_modes() {
    if Command::new("rg").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "ς\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
        .arg("bench")
        .arg(dir.path())
        .arg("--query=σ")
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let comparison = &report["cli_comparison"];
    assert_eq!(comparison["comparable"], false);
    assert!(
        comparison["reason"]
            .as_str()
            .unwrap()
            .contains("result mismatch")
    );
    for mode in ["repolens_cli", "repolens_mcp", "rg_cli"] {
        assert!(
            comparison[format!("{mode}_ms")]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(comparison[format!("{mode}_summary")].is_null());
    }
}

#[test]
fn missing_rg_is_not_reported_as_a_measurement() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_repolens"))
        .env("PATH", "")
        .arg("bench")
        .arg(dir.path())
        .args(["--query=needle", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["cli_comparison"]["comparable"], false);
    assert!(
        report["cli_comparison"]["reason"]
            .as_str()
            .unwrap()
            .contains("cannot launch")
    );
    assert!(
        report["cli_comparison"]["rg_cli_ms"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
