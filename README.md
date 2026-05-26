# RepoLens

Fast local codebase index for AI agents.

## MVP

```powershell
cargo run -- index .
cargo run -- tree .
cargo run -- search "fn main" .
cargo run -- read src/main.rs . --lines 1-40
cargo run -- status .
cargo run -- word main .
cargo run -- outline src/main.rs .
cargo run -- symbol ProjectIndex .
cargo run -- deps src/main.rs .
```

## Direction

- Cross-platform first: Windows, Linux, macOS.
- Compact agent tools: tree, search, read, symbols, deps, MCP.
- Safe defaults: respect `.gitignore`, skip heavy folders, block unsafe paths.
