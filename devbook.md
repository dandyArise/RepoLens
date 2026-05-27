# RepoLens Dev Book

## Objectif

Créer un moteur local d'intelligence de code pour agents IA.

RepoLens doit fournir :

- indexation rapide d'un repo;
- recherche texte et symboles;
- lecture ciblée de fichiers;
- outline structurel;
- graphe de dépendances;
- snapshot persistant;
- serveur MCP stdio;
- serveur HTTP local optionnel;
- compatibilité Windows, Linux, macOS et Unix-like dès le départ.

## Non-objectifs v1

- Pas d'IDE complet.
- Pas de LSP complet.
- Pas d'analyse sémantique profonde.
- Pas d'auth distante.
- Pas de cloud.
- Pas de base SQL obligatoire.
- Pas de support parfait de tous les langages.

## Stack Rust

Crates de base :

- `clap` : CLI.
- `ignore` : walker respectant `.gitignore`.
- `camino` : paths UTF-8.
- `dunce` : canonicalisation Windows-friendly.
- `serde`, `serde_json` : snapshot v1.
- `bincode` : snapshot binaire v2.
- `rayon` : indexation parallèle.
- `roaring` : bitmaps rapides.
- `memmap2` : mmap snapshot/index.
- `tree-sitter` : symboles. Rust, TypeScript, JavaScript, TSX et Python sont branchés en premier.
- `notify` : watcher.
- `tokio`, `axum` : HTTP local.
- `ropey` : édition texte.
- `parking_lot` : locks.
- `thiserror`, `anyhow` : erreurs.
- `tracing` : logs.

## Architecture Cible

```text
CLI / MCP / HTTP
      |
   Service
      |
 ProjectIndex
      |
+-----+------+---------+---------+
| files      | symbols | search  |
| deps       | lines   | watcher |
+------------+---------+---------+
      |
 SnapshotStore
```

Modules cibles :

```text
src/
  main.rs
  cli.rs
  service.rs
  project.rs
  scanner.rs
  file_table.rs
  line_index.rs
  search/
  parser/
  symbols.rs
  deps.rs
  snapshot.rs
  mcp.rs
  http.rs
  edit.rs
  config.rs
  security.rs
```

## Modèle De Données

```rust
type FileId = u32;

struct FileMeta {
    id: FileId,
    path: Utf8PathBuf,
    lang: Language,
    size: u64,
    mtime_ms: u64,
    hash: u64,
    line_count: u32,
}

struct Location {
    file_id: FileId,
    line: u32,
    column: u32,
}

struct Symbol {
    name: String,
    kind: SymbolKind,
    loc: Location,
    end_line: u32,
}

struct ProjectIndex {
    files: Vec<FileMeta>,
    path_to_id: HashMap<Utf8PathBuf, FileId>,
    symbols_by_name: HashMap<String, Vec<Symbol>>,
    words: HashMap<String, RoaringBitmap>,
    trigrams: HashMap<[u8; 3], RoaringBitmap>,
    deps_forward: Vec<RoaringBitmap>,
    deps_reverse: Vec<RoaringBitmap>,
}
```

## Règles Cross-Platform

- Aucun `/tmp`.
- Aucun séparateur `/` ou `\` codé en dur.
- Utiliser `camino::Utf8Path` et `Utf8PathBuf`.
- Supporter `C:\path`, espaces et Unicode.
- Utiliser `std::fs` ou crates portables.
- Tester Windows, Linux, macOS en CI.
- Config :
  - Windows : `%APPDATA%`.
  - macOS : `~/Library/Application Support`.
  - Linux/Unix : `$XDG_CONFIG_HOME` ou `~/.config`.
- Installers :
  - Windows : `install.ps1`.
  - Linux/macOS : `install.sh`.

## Sécurité

Bloquer ou ignorer par défaut :

- `.env`
- `.env.*`
- `*.pem`
- `*.key`
- `id_rsa`
- `id_ed25519`
- `credentials*`
- `secrets*`
- `.git`
- `node_modules`
- `target`
- `dist`
- `build`
- `.next`
- `.cache`

Config cible `.repolensrc.toml` :

```toml
include = []
exclude = []
max_file_size = "1mb"
allow_sensitive = false
```

## CLI Cible

```powershell
repolens index .
repolens tree .
repolens search "handleAuth" .
repolens word UserService .
repolens symbol UserService .
repolens outline src/main.rs .
repolens read src/main.rs . --lines 1-80
repolens deps src/main.rs .
repolens status .
repolens serve .
repolens mcp .
```

## MCP Tools Cibles

- `repolens_tree`
- `repolens_outline`
- `repolens_symbol`
- `repolens_search`
- `repolens_word`
- `repolens_read`
- `repolens_edit`
- `repolens_deps`
- `repolens_hot`
- `repolens_changes`
- `repolens_status`
- `repolens_snapshot`
- `repolens_bundle`
- `repolens_index`

## HTTP Local Cible

Bind uniquement :

- `127.0.0.1`
- `::1`

Routes :

- `GET /status`
- `GET /tree`
- `GET /search?q=`
- `GET /word?q=`
- `GET /symbol?q=`
- `GET /outline?path=`
- `GET /read?path=&lines=`
- `GET /deps?path=`
- `POST /edit`

## Performance Cible

Sur repo moyen 100k lignes :

- index initial : moins de 3s;
- recherche warm : moins de 10ms;
- symbol lookup : moins de 5ms;
- read range : moins de 5ms;
- snapshot load : moins de 300ms.

Bench contre :

- `ripgrep`
- `grep`
- `ast-grep`

État actuel :

- [x] Commande `bench` pour `index/search/symbol`.
- [x] Comparaison `rg` optionnelle si disponible.

## Checklist

### Phase 0: Fondation

- [x] Créer repo local `RepoLens`.
- [x] Initialiser projet Rust.
- [x] Ajouter README.
- [x] Ajouter `devbook.md`.
- [x] Ajouter `.gitignore`.
- [x] Valider `cargo check`.
- [x] Valider `cargo fmt --check`.
- [ ] Commit initial.
- [ ] Push vers `https://github.com/dandyArise/RepoLens.git`.

### Phase 1: Core CLI

- [x] Commande `index`.
- [x] Commande `tree`.
- [x] Commande `search`.
- [x] Commande `read`.
- [x] Respect `.gitignore`.
- [x] Ignorer dossiers lourds (`target`, `node_modules`, `.git`, etc.).
- [x] Snapshot JSON `.repolens/index.json`.
- [x] Refactor `cli.rs`.
- [x] Refactor `index.rs`.
- [x] Refactor `snapshot.rs`.
- [x] Refactor `search.rs`.
- [x] Refactor `read.rs`.
- [x] Ajouter `status`.
- [x] Ajouter limite `max_bytes` à `read`.
- [x] Ajouter vérification hash optionnelle à `read`.

### Phase 2: Index Rapide

- [x] Introduire `FileId`.
- [x] Stocker `mtime_ms`.
- [x] Stocker `size`.
- [x] Stocker `hash`.
- [x] Ajouter line offsets.
- [x] Ajouter tokenizer mots.
- [x] Ajouter word index.
- [x] Ajouter trigram extraction.
- [x] Ajouter trigram index.
- [x] Ajouter recherche par candidats trigram.
- [x] Vérifier match réel après candidats.
- [x] Ajouter commande `word`.
- [x] Ajouter tests word/trigram.

### Phase 3: Sécurité

- [x] Centraliser règles dans `security.rs`.
- [x] Bloquer `.env`.
- [x] Bloquer clés privées.
- [x] Bloquer credentials/secrets.
- [x] Bloquer path traversal.
- [x] Refuser paths absolus dans commandes ciblant le repo.
- [x] Ajouter `.repolensrc.toml`.
- [x] Ajouter `allow_sensitive = false`.
- [ ] Tests sécurité Windows/Linux.

### Phase 4: Tests

- [x] Tests path normalization.
- [x] Tests scan `.gitignore`.
- [x] Tests binary detection.
- [x] Tests line range.
- [x] Tests search.
- [x] Tests snapshot load/save.
- [ ] Fixtures Rust.
- [ ] Fixtures TypeScript.
- [ ] Fixtures Windows paths.
- [x] Ajouter `cargo test` CI.

### Phase 5: CI Cross-Platform

- [x] GitHub Actions `windows-latest`.
- [x] GitHub Actions `ubuntu-latest`.
- [x] GitHub Actions `macos-latest`.
- [x] `cargo fmt --check`.
- [x] `cargo clippy`.
- [x] `cargo test`.
- [x] Build release Windows.
- [x] Build release Linux x86_64.
- [ ] Build release Linux arm64.
- [x] Build release macOS x86_64.
- [x] Build release macOS arm64.

### Phase 6: Symboles

- [x] Ajouter `tree-sitter`.
- [x] Parser Rust.
- [x] Parser TypeScript.
- [x] Parser JavaScript.
- [x] Parser Python.
- [ ] Parser Go.
- [ ] Parser PHP.
- [x] Parser TSX.
- [x] Commande `outline`.
- [x] Commande `symbol`.
- [x] Index `symbols_by_name`.
- [x] Tests symboles.

### Phase 7: Dépendances

- [x] Modèle `ImportRef`.
- [x] Résolution TS/JS relative.
- [x] Résolution Rust basique.
- [x] Résolution Python best-effort.
- [ ] Résolution Go best-effort.
- [x] `deps_forward`.
- [x] `deps_reverse`.
- [x] Commande `deps`.
- [x] Commande `rdeps`.
- [x] Tests deps.

### Phase 8: MCP

- [x] Serveur JSON-RPC 2.0 stdio.
- [x] Tool `repolens_tree`.
- [x] Tool `repolens_search`.
- [x] Tool `repolens_read`.
- [x] Tool `repolens_word`.
- [x] Tool `repolens_status`.
- [ ] Tool `repolens_snapshot`.
- [x] Tool `repolens_bundle`.
- [x] Tool `repolens_outline`.
- [x] Tool `repolens_symbol`.
- [x] Tool `repolens_deps`.
- [x] Tool `repolens_rdeps`.
- [x] Tests MCP request/response.

### Phase 9: HTTP Local

- [ ] Ajouter `tokio`.
- [ ] Ajouter `axum`.
- [ ] Route `/status`.
- [ ] Route `/tree`.
- [ ] Route `/search`.
- [ ] Route `/word`.
- [ ] Route `/read`.
- [ ] Route `/outline`.
- [ ] Route `/symbol`.
- [ ] Route `/deps`.
- [ ] Bind localhost only.
- [ ] Tests HTTP.

### Phase 10: Edition

- [ ] Ajouter `ropey`.
- [x] `replace lines`.
- [x] `insert before line`.
- [x] `delete lines`.
- [x] Vérifier hash/version avant write.
- [x] Ecriture fichier temporaire.
- [x] Rename atomique.
- [x] Réindexer fichier après edit.
- [x] Commande `edit`.
- [x] Tool MCP `repolens_edit`.
- [x] Tests edit ranges.

### Phase 11: Watcher

- [x] Ajouter `notify`.
- [ ] Fallback polling.
- [x] Sequence number global.
- [x] Commande `watch`.
- [x] Commande `changes`.
- [x] Commande `hot`.
- [ ] Réindex incrémental fin.
- [x] Tool MCP `repolens_changes`.
- [x] Tests watcher.

### Phase 12: Release

- [x] Installer Windows `install.ps1`.
- [x] Installer Linux/macOS `install.sh`.
- [x] Checksums SHA256.
- [x] Archives zip/tar.gz.
- [x] Auto-config Codex.
- [x] Auto-config Claude.
- [x] Auto-config Cursor.
- [ ] Documentation installation.

## Prochaine Étape

Priorité immédiate : terminer Phase 1 et Phase 2.

Ordre recommandé :

1. Ajouter HTTP local.
2. Ajouter Linux arm64 release.
3. Ajouter `repolens_snapshot`.
4. Ajouter fallback polling watcher.
5. Ajouter documentation installation.
