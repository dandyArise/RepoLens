use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_repolens"))
}

#[test]
fn concurrent_processes_cannot_commit_the_same_guard_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    let text = format!("original\n{}", "padding\n".repeat(250_000));
    fs::write(&path, &text).unwrap();
    fs::write(
        dir.path().join(".repolensrc.toml"),
        "max_file_size = '20mb'\n",
    )
    .unwrap();
    let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
    let children = (0..4)
        .map(|writer| {
            let mut command = binary();
            command
                .arg("edit")
                .arg("sample.txt")
                .arg(dir.path())
                .args(["--op", "replace", "--start", "1", "--content"])
                .arg(format!("writer_{writer}\n"))
                .args(["--hash", &hash])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            (writer, command.spawn().unwrap())
        })
        .collect::<Vec<_>>();
    let mut winner = None;
    for (writer, child) in children {
        let output = child.wait_with_output().unwrap();
        if output.status.success() {
            assert!(
                winner.replace(writer).is_none(),
                "more than one process committed"
            );
        } else {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("hash mismatch"),
                "{output:?}"
            );
        }
    }
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .starts_with(&format!("writer_{}\n", winner.unwrap()))
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join(".repolens/index.json")).unwrap())
            .unwrap();
    let file = saved["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "sample.txt")
        .unwrap();
    assert_eq!(
        file["hash"],
        blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
    );
}

#[test]
fn codex_home_enable_status_and_mcp_discovery_work_together() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let custom = dir.path().join("custom-codex");
    let root = dir.path().join("repo");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), "fn sample() {}\n").unwrap();
    let command = || {
        let mut cmd = binary();
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("CODEX_HOME", &custom)
            .env_remove("REPOLENS_MCP_ROOTS");
        cmd
    };
    assert!(
        command()
            .args(["enable", "--target", "codex"])
            .arg(&root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let config_path = custom.join("config.toml");
    let raw = fs::read_to_string(&config_path).unwrap();
    assert!(!home.join(".codex/config.toml").exists());
    let mut config: toml::Value = toml::from_str(&raw).unwrap();
    let server = config["mcp_servers"]
        .as_table_mut()
        .unwrap()
        .iter_mut()
        .next()
        .unwrap()
        .1;
    server["enabled"] = toml::Value::Boolean(false);
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let status = command()
        .args(["mcp-status", "--target", "codex"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("Codex: disabled"));
    assert!(
        command()
            .args(["enable", "--target", "codex"])
            .arg(&root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let status = command()
        .args(["mcp-status", "--target", "codex"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("enabled (1 project)"));

    let mut child = command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"repolens_status","arguments":{"workspaceRoot": root}}});
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(response["result"]["isError"], true, "{response}");
    let status: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        status["root"],
        dunce::canonicalize(root).unwrap().to_str().unwrap()
    );
}
