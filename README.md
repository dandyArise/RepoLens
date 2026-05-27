# RepoLens 🔎

Fast local codebase index for AI agents.

RepoLens scans a repository, builds compact local indexes, and exposes code navigation tools through a CLI, an MCP stdio server, and a localhost HTTP API/UI. It is designed to be cross-platform from the start: Windows, Linux, macOS, and Unix-like environments.

## Links

- 🚀 [Latest release](https://github.com/dandyArise/RepoLens/releases/latest)
- 📦 [v0.1.0 binaries](https://github.com/dandyArise/RepoLens/releases/tag/v0.1.0)
- ⚙️ [GitHub Actions builds](https://github.com/dandyArise/RepoLens/actions)
- 🧩 [Windows installer](https://github.com/dandyArise/RepoLens/blob/main/install/install.ps1)
- 🧩 [Linux/macOS installer](https://github.com/dandyArise/RepoLens/blob/main/install/install.sh)

## Highlights

- ⚡ Fast Rust CLI with parallel indexing.
- 🧠 MCP stdio server for AI agents.
- 🖥️ Localhost HTTP API and optional UI.
- 🧭 Symbols, search, outlines, deps, reverse deps, and guarded reads.
- ✍️ Safe edit tool with hash guard, atomic write, and immediate reindex.
- 👀 Watch mode with incremental updates and change tracking.
- 🧰 Auto-config for Codex, Claude Desktop, and Cursor.
- 🔐 Sensitive files are blocked by default.
- 🌍 Release binaries for Windows, Linux, and macOS.

## Status

Implemented:

- Cross-platform Rust CLI.
- `.gitignore`-aware scanner.
- Safe defaults for sensitive files.
- JSON snapshot at `.repolens/index.json`.
- Binary snapshot at `.repolens/index.bin` with mmap loading for faster reloads.
- Snapshot metadata command/tool.
- Word and trigram indexes for search.
- Parallel file indexing.
- Symbol extraction for Rust, TypeScript, JavaScript, TSX/JSX, Python, Go, PHP, Java, C#, C/C++, Ruby, JSON, YAML, and TOML.
- Import/dependency extraction for Rust, TS/JS, Python, Go, PHP, Java, C#, C/C++, and Ruby.
- Relative TS/JS dependency resolution.
- Forward and reverse dependency graph for resolved imports.
- MCP stdio server with compact tools.
- Localhost HTTP API.
- Optional localhost UI at `/`.
- Watch mode with change sequence tracking and incremental file updates.
- Basic benchmark command with optional `rg` comparison.
- Release builds for Windows, Linux, and macOS.
- CI for Windows, Linux, and macOS with Rust cache.

Not implemented yet:

- perf report for large repositories.

## Install From Release 📦

Download binaries from [GitHub Releases](https://github.com/dandyArise/RepoLens/releases/latest).

Windows PowerShell:

```powershell
iwr https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.ps1 -UseBasicParsing | iex
```

Linux/macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.sh | sh
```

Defaults:

- Windows installs to `%USERPROFILE%\bin`
- Linux/macOS installs to `$HOME/.local/bin`
- checksums are verified before copying the binary
- supported assets: Windows x86_64, Linux x86_64/arm64, macOS x86_64/arm64

Installer options:

```powershell
.\install\install.ps1 -Version v0.1.0 -InstallDir "$env:USERPROFILE\bin"
.\install\install.ps1 -Init -InitTarget codex -InitRoot "$PWD"
.\install\install.ps1 -Action update
.\install\install.ps1 -Action uninstall
```

```sh
REPOLENS_VERSION=v0.1.0 REPOLENS_INSTALL_DIR="$HOME/.local/bin" sh install/install.sh
REPOLENS_INIT=1 REPOLENS_INIT_TARGET=codex REPOLENS_INIT_ROOT="$PWD" sh install/install.sh
REPOLENS_ACTION=update sh install/install.sh
REPOLENS_ACTION=uninstall sh install/install.sh
```

After install, make sure the install directory is in `PATH`, then run:

```powershell
repolens --help
repolens init . --target codex
```

## Use With AI Agents 🤖

RepoLens can register itself as an MCP server for Codex, Claude Desktop, and Cursor.

From the repository you want to index:

```powershell
repolens init . --target all
```

Or configure one client only:

```powershell
repolens init . --target codex
repolens init . --target claude
repolens init . --target cursor
```

Then restart your AI client. The client will receive a `repolens` MCP server that runs:

```text
repolens mcp <repo-root>
```

For best results in agent workflows, add a short note to your project instructions:

```md
Use RepoLens MCP tools first when available:
- repolens_search
- repolens_symbol
- repolens_outline
- repolens_deps
- repolens_read
- repolens_bundle
```

## Install From Source 🛠️

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

## Release Builds 🚀

Release workflow targets:

- `repolens-windows-x86_64.zip`
- `repolens-linux-x86_64.tar.gz`
- `repolens-linux-arm64.tar.gz`
- `repolens-darwin-x86_64.tar.gz`
- `repolens-darwin-arm64.tar.gz`

Each archive is accompanied by a `.sha256` file. Tagged releases publish a combined `checksums.sha256`.

Published assets are available on the [latest release page](https://github.com/dandyArise/RepoLens/releases/latest).

To create a release:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

During development:

```powershell
cargo run -- --help
```

## Quick Start ⚡

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

## CLI Commands 🧰

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
root: C:\path\to\project
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
- Go
- PHP
- Java
- C#
- C/C++
- Ruby
- JSON/TOML/YAML keys

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
- Go: `import`
- PHP: `use`, `require`, `include`
- Java: `import`
- C#: `using`
- C/C++: `#include`
- Ruby: `require`, `require_relative`

### `bench`

Measures index/search/symbol timings and compares search with `rg` if available.

```powershell
repolens bench . --query ProjectIndex --symbol ProjectIndex --limit 20
repolens bench . --query ProjectIndex --symbol ProjectIndex --json
```

The report includes build/save/load timings and JSON/binary snapshot sizes.

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

### `serve`

Starts the localhost HTTP API. Non-loopback hosts are refused.

```powershell
repolens serve .
repolens serve . --host 127.0.0.1 --port 4177
```

Routes:

- `GET /`
- `GET /status`
- `GET /snapshot`
- `GET /tree?limit=200`
- `GET /search?q=ProjectIndex&limit=20`
- `GET /word?q=ProjectIndex&limit=20`
- `GET /read?path=src/main.rs&lines=1-40`
- `GET /outline?path=src/main.rs`
- `GET /symbol?q=ProjectIndex&limit=20`
- `GET /deps?path=src/main.rs`
- `GET /rdeps?path=src/main.rs`
- `GET /changes`
- `POST /edit`

### `snapshot`

Prints `.repolens/index.json` metadata.

```powershell
repolens snapshot .
```

### `watch`, `changes`, `hot`

`watch` keeps `.repolens/index.json` fresh and writes `.repolens/changes.json`. Modified, created, and removed files are applied incrementally when possible.

```powershell
repolens watch .
repolens watch . --poll --interval-ms 1000
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

## MCP Tools 🤖

Implemented tools:

- `repolens_status`
- `repolens_snapshot`
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

## Configuration ⚙️

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

## Safety Defaults 🔐

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

## Development 🧪

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
- HTTP loopback guard
- snapshot save/load
- binary snapshot save/load
- MCP request handling

## Roadmap 🗺️

Next planned work:

- perf report for large repositories.

Longer term:

- More robust MCP auto-config variants.
