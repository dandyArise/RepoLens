# RepoLens Roadmap

RepoLens doit ameliorer la comprehension generique des depots. Les stacks comme Python, Node, Rust ou Go sont des adaptateurs branches sur un socle commun, pas le centre du design.

## Design Principles

- Build a repo-generic foundation first: files, docs, configs, git, CI, commands, dependencies, tests, and cross-cutting risks.
- Add stack adapters on top of that foundation without changing the core model.
- Prefer structured parsers for universal formats before stack-specific rules.
- Surface facts, inferences, and suspected issues with clear confidence levels.
- Keep outputs compact enough for AI agents to use directly.

## Architecture Layers

| Layer | Role |
|---|---|
| Generic foundation | Files, docs, configs, git, CI, commands, dependencies |
| Repository analysis | Architecture, manifests, tests, scripts, transversal risks |
| Stack adapters | Node, Python, Rust, Go, Java, .NET, PHP, Ruby |
| Extensible rules | Stack-specific checks added without breaking the foundation |

## Usage Feedback To Fix Generically

| Observation | Generic fix |
|---|---|
| Code outlines help identify which files deserve deeper reads | Keep improving outlines and make them available across more languages and file types |
| TOML files can fall back to weak line-based reads | Add native TOML parsing for manifests, tool configs, sections, dependencies, and scripts |
| Markdown and config files are not structured enough | Add section-aware Markdown and config readers with precise extraction by heading/key/path |
| Test files still require full manual reads | Add generic `tests-aware` analysis for frameworks, fixtures, mocks, assertions, and expected behavior |
| Git state and test execution still happen outside RepoLens | Add lightweight git/diff awareness, while keeping command execution as explicit validation outside automatic analysis |

## Priority Roadmap

| Priority | RepoLens task | Generic objective |
|---|---|---|
| P0 | Add native TOML support | Read project and tool configs across ecosystems |
| P0 | Add native YAML support | Read CI, app configs, manifests, and examples |
| P0 | Add structured Markdown support | Extract install, usage, commands, architecture, and test sections |
| P0 | Create `repolens health .` | Produce a transversal repository health report |
| P0 | Detect project manifests | Identify `package.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`, `pom.xml`, and similar files |
| P0 | Detect dependency inconsistencies | Compare manifests, lockfiles, imports/usages, and scripts |
| P0 | Detect project commands | Extract install, dev, test, lint, build, format, and typecheck commands |
| P0 | Create generic `tests-aware` mode | Identify test frameworks, test files, fixtures, and important assertions |
| P1 | Detect missing referenced files | Find dead paths in scripts, docs, and configs |
| P1 | Detect duplicated or divergent configs | Find competing sources of truth |
| P1 | Detect fragile imports or loads | Flag code that loads config, network, files, or heavy dependencies too early |
| P1 | Add lightweight git integration | Report branch, remote, modified files, and sensitive ignored files |
| P1 | Add `diff-aware` mode | Summarize current changes and their risks |
| P1 | Detect CI/CD | Read GitHub Actions, GitLab CI, Azure Pipelines, CircleCI, and similar systems |
| P1 | Detect repository architecture | Identify monorepo, single package, apps/libs, and services |
| P2 | Add confidence scoring | Separate confirmed facts, inferences, and suspicions |
| P2 | Improve snippets with local context | Show the useful block, function, or section without reading the whole file |
| P2 | Add generic security rules | Detect secrets, `.env`, credentials, and production configs |
| P2 | Add ecosystem adapters | Add Node, Python, Rust, Go, Java, .NET, PHP, and Ruby rules |
| P2 | Add prioritized action report | Generate an impact-ranked todo list |

## Recommended Order

1. Universal formats: JSON, TOML, YAML, Markdown, XML.
2. Manifest, command, and CI detection.
3. `repolens health .`.
4. Generic `tests-aware` mode.
5. `diff-aware` and git integration.
6. Stack adapters, starting with the most common ecosystems.

## First Implementation Slice

The first useful slice should stay small and generic:

1. Native TOML reader for manifests and tool configs.
2. Native YAML reader for CI and config files.
3. Structured Markdown section extraction.
4. Manifest inventory across common ecosystems.
5. Project command extraction from manifests and docs.
6. Initial `repolens health .` report using only high-confidence findings.
