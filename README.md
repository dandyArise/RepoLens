# RepoLens

Fast local codebase index for AI agents.

RepoLens scans a repository, builds compact local indexes, and exposes code navigation tools through both a CLI and an MCP stdio server. It is designed to be cross-platform from the start: Windows, Linux, macOS, and Unix-like environments.

## Status

Implemented today:

- Cross-platform Rust CLI.
- `.gitignore`-aware scanner.
- Safe defaults for sensitive files.
- JSON snapshot at `.repolens/index.json`.
- Word and trigram indexes for search.
- Symbol extraction with tree-sitter for Rust, TypeScript, JavaScript, TSX/JSX, and Python.
- Import/dependency extraction for Rust, TS/JS, and Python.
- Relative TS/JS dependency resolution.
- Forward and reverse dependency graph for resolved imports.
- MCP stdio server with compact tools.
- Watch mode with change sequence tracking.
- Basic benchmark command with optional `rg` comparison.
- CI for Windows, Linux, and macOS.

Not implemented yet:

- HTTP local server.
- Fine-grained incremental reindex.

## Install From Source

Requirements:

- Rust stable.
- `rg` optional, only used by `bench` for comparison.

```powershell
git clone https://github.com/dandyArise/RepoLens.git
cd RepoLens
cargo build --release
```

Binary:

```powershell
.\target\release\repolens.exe --help
```

## Release Builds

Release workflow targets:

- `repolens-windows-x86_64.zip`
- `repolens-linux-x86_64.tar.gz`
- `repolens-darwin-x86_64.tar.gz`
- `repolens-darwin-arm64.tar.gz`

Each archive is accompanied by a `.sha256` file. Tagged releases publish a combined `checksums.sha256`.

To create a release:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

During development:

```powershell
cargo run -- --help
```

## Install From Release

Windows PowerShell:

```powershell
iwr https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.ps1 -UseBasicParsing | iex
```

Linux/macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.sh | sh
```

Installer options:

```powershell
.\install\install.ps1 -Version v0.1.0 -InstallDir "$env:USERPROFILE\bin"
```

```sh
REPOLENS_VERSION=v0.1.0 REPOLENS_INSTALL_DIR="$HOME/.local/bin" sh install/install.sh
```

## Quick Start

```powershell
cargo run -- index .
cargo run -- status .
cargo run -- search "ProjectIndex" .
cargo run -- read src/main.rs . --lines 1-40
cargo run -- outline src/main.rs .
cargo run -- symbol ProjectIndex .
cargo run -- deps src/main.rs .
cargo run -- bench . --query ProjectIndex --symbol ProjectIndex
```

## CLI Commands

### `index`

Builds `.repolens/index.json`.

```powershell
repolens index .
```

### `status`

Prints index counts.

```powershell
repolens status .
```

Example:

```text
root: C:\Users\dandy\Documents\Github\RepoLens
files: 20
words: 1153
trigrams: 8889
symbols: 136
symbol names: 120
deps files: 14
```

### `tree`

Lists indexed files with line and byte counts.

```powershell
repolens tree .
```

### `search`

Searches indexed content.

```powershell
repolens search "handleAuth" .
repolens search "ProjectIndex" . --limit 10
```

### `word`

Finds files containing an identifier-like word.

```powershell
repolens word UserService .
```

### `read`

Reads a file with optional line range, byte cap, and hash guard.

```powershell
repolens read src/main.rs . --lines 1-80
repolens read src/main.rs . --max-bytes 4000
repolens read src/main.rs . --hash <blake3_hash>
```

### `outline`

Lists symbols in one file.

```powershell
repolens outline src/main.rs .
repolens outline src/lib.ts .
repolens outline app.py .
```

Supported symbol languages:

- Rust
- TypeScript
- JavaScript
- TSX/JSX
- Python

### `symbol`

Finds symbols by name. Exact normalized lookup is indexed; substring fallback is kept.

```powershell
repolens symbol ProjectIndex .
repolens symbol make_user . --limit 5
```

### `deps`

Lists imports/dependencies found in one file.

```powershell
repolens deps src/main.rs .
repolens deps src/app.ts .
repolens deps main.py .
repolens rdeps src/utils.ts .
```

Currently extracts:

- Rust: `use`, `mod`
- TS/JS: `import`, `require`, with relative path resolution for `./` and `../`
- Python: `import`, `from ... import`

### `bench`

Measures index/search/symbol timings and compares search with `rg` if available.

```powershell
repolens bench . --query ProjectIndex --symbol ProjectIndex --limit 20
```

### `edit`

Applies a guarded line edit and immediately rebuilds `.repolens/index.json`.

The current file hash is required. Get it from `tree`, `.repolens/index.json`, or MCP `repolens_tree`.

```powershell
repolens edit src/main.rs . --op replace --start 10 --end 12 --content "new text`n" --hash <current_hash>
repolens edit src/main.rs . --op insert --start 20 --content "inserted line`n" --hash <current_hash>
repolens edit src/main.rs . --op delete --start 30 --end 35 --hash <current_hash>
```

Safety rules:

- refuses missing or stale hashes;
- refuses path traversal;
- refuses sensitive paths unless `allow_sensitive = true`;
- refuses non-UTF-8 files;
- writes through a temp file and atomic rename;
- reindexes immediately after write.

### `mcp`

Starts the MCP stdio server.

```powershell
repolens mcp .
```

### `watch`, `changes`, `hot`

`watch` keeps `.repolens/index.json` fresh and writes `.repolens/changes.json`.

```powershell
repolens watch .
repolens changes .
repolens hot . --limit 10
```

`changes` returns the latest watcher sequence and changed paths. `hot` prints only the most recent changed paths.

### `init`

Writes MCP configuration for supported clients.

```powershell
repolens init . --target all
repolens init . --target codex
repolens init . --target claude
repolens init . --target cursor
```

Current targets:

- Codex: `~/.codex/config.toml`
- Claude Desktop:
  - Windows: `%APPDATA%\Claude\claude_desktop_config.json`
  - macOS/Linux fallback: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Cursor: `~/.cursor/mcp.json`

`init` registers the current `repolens` executable with:

```text
repolens mcp <repo-root>
```

## MCP Tools

Implemented tools:

- `repolens_status`
- `repolens_tree`
- `repolens_search`
- `repolens_word`
- `repolens_read`
- `repolens_outline`
- `repolens_symbol`
- `repolens_deps`
- `repolens_rdeps`
- `repolens_edit`
- `repolens_changes`
- `repolens_bundle`

Example JSON-RPC call:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"repolens_status","arguments":{}}}
```

Bundle example:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"repolens_bundle","arguments":{"ops":[{"tool":"repolens_status","arguments":{}},{"tool":"repolens_search","arguments":{"query":"ProjectIndex","limit":3}}]}}}
```

## Configuration

RepoLens reads `.repolensrc.toml` from the repository root when present.

Example:

```toml
max_file_size = "1mb"
allow_sensitive = false
```

Defaults:

- `max_file_size = "1mb"`
- `allow_sensitive = false`

An example file is included:

```powershell
copy .repolensrc.example.toml .repolensrc.toml
```

## Safety Defaults

RepoLens skips heavy or generated folders:

- `.git`
- `.repolens`
- `node_modules`
- `target`
- `dist`
- `build`
- `.next`
- `.cache`
- `.venv`
- `__pycache__`

RepoLens blocks sensitive names by default:

- `.env`
- `.env.*`
- `*.pem`
- `*.key`
- `id_rsa`
- `id_ed25519`
- `credentials*`
- `secrets*`

Set `allow_sensitive = true` only for controlled local testing.

## Development

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Current test coverage includes:

- path safety
- scanner and `.gitignore`
- binary detection
- config parsing
- word/trigram extraction
- symbol extraction
- dependency extraction
- TS/JS relative dependency resolution
- forward/reverse dependency graph
- watcher state tracking
- snapshot save/load
- MCP request handling

## Roadmap

Next planned work:

- HTTP localhost server.
- Fine-grained incremental reindex.
- Linux arm64 release build.

Longer term:

- More languages.
- Faster binary snapshots.
- Parallel indexing.
- More robust MCP auto-config variants.
