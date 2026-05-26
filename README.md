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
- MCP stdio server with compact tools.
- Basic benchmark command with optional `rg` comparison.
- CI for Windows, Linux, and macOS.

Not implemented yet:

- Release binaries.
- Install scripts.
- HTTP local server.
- Atomic edit command.
- Watcher/incremental reindex.
- Deep dependency resolution/reverse deps.

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

During development:

```powershell
cargo run -- --help
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
```

Currently extracts:

- Rust: `use`, `mod`
- TS/JS: `import`, `require`
- Python: `import`, `from ... import`

### `bench`

Measures index/search/symbol timings and compares search with `rg` if available.

```powershell
repolens bench . --query ProjectIndex --symbol ProjectIndex --limit 20
```

### `mcp`

Starts the MCP stdio server.

```powershell
repolens mcp .
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
- snapshot save/load
- MCP request handling

## Roadmap

Next planned work:

- TS/JS relative dependency resolution.
- Release workflow with Windows/Linux/macOS binaries.
- PowerShell and shell installers.
- Atomic edit command.
- Watcher and incremental reindex.
- HTTP localhost server.

Longer term:

- More languages.
- Reverse dependency graph.
- Faster binary snapshots.
- Parallel indexing.
- MCP auto-config for Codex/Claude/Cursor.
