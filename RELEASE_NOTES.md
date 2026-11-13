# RepoLens v0.1.23

## Automatic MCP index updates

- Cached MCP workspaces now watch file changes and apply queued updates before tool calls. Creation, modification, deletion and renames no longer require a manual index command.
- Startup reconciles changes made while the server was stopped. Ignore-rule changes, directory events and polling use conservative reconciliation; native watcher failures fall back to polling.
- Isolated file events update only affected files, with a targeted traversal that preserves the full scanner's ignore, hidden-file, sensitive-path, symlink and size policies.
- Watchers are released when cached workspaces are evicted or access is revoked. MCP updates stay in memory and do not rewrite snapshots.
- Filesystem notifications are asynchronous: a call issued immediately after a write can precede notification delivery. Inherited ignore rules and polling are checked on requests at two-second intervals.

## Faster persistent MCP search

- Buffer JSON-RPC output and serialize search hits directly as compact JSON.
- Parallelize large searches while retaining deterministic ordering and result limits. Small searches keep the sequential path.
- Reduce repeated allocations during case-insensitive line matching while preserving Unicode behavior.

## Comparable three-mode benchmarks

- Compare CLI, persistent MCP and ripgrep over the exact same indexed file set and all matching lines.
- Validate normalized results during warmup and every measured round; reject comparisons if the corpus or snapshots change.
- Report seven rotating-order timing samples, summaries, persistent MCP startup and validation status.
- Use a temporary ripgrep whitelist instead of a long file argument list, avoiding Windows command-line length limits.

## Local validation and performance

The local Windows test suite passed 160 tests. On a ComfyUI corpus of 1,226 indexed files, all seven benchmark queries matched ripgrep. These are local measurements, not cross-platform performance guarantees.

For a 56-byte Python probe, seven measured create/modify/delete cycles after warmup produced the following median first-search times. Both versions were measured on the same machine; the MCP process remained alive and no manual indexing was performed.

| Operation | Previous automatic updates | Optimized updates | ripgrep |
| --- | ---: | ---: | ---: |
| Create | 100.71 ms | 26.52 ms | 41.66 ms |
| Modify (same length) | 100.51 ms | 27.32 ms | 41.09 ms |
| Delete | 72.99 ms | 22.22 ms | 43.44 ms |

All 21 optimized first responses matched ripgrep. Subsequent unchanged MCP searches took approximately 1.2–1.4 ms. Large batches and directory or ignore-rule changes can still require full reconciliation.

## Upgrade

Install the archive for your platform using the existing installer or replace the executable. Restart MCP clients after upgrading so existing processes load the new binary. Archives are provided for Windows x64, Linux x64/ARM64 and macOS Intel/ARM64, with SHA-256 checksums.
