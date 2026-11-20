# Fase 1: storage SurrealDB → SQLite (schema CRG v15) + petgraph

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `codegraph index`, `codegraph tool run …` e `codegraph start stdio` funcionando sobre
`<repo>/.code-review-graph/graph.db` (schema do code-review-graph v15), sem SurrealDB e sem
`libssl` no binário.

**Architecture:** O crate `codegraph-graph` é reescrito no lugar: ganha `sqlite_store`
(abre/cria o banco CRG v15, grava `CodeNode`/`CodeEdge`, metadados de arquivo/projeto,
reconciliação) e `graph_query` (snapshot `petgraph` em memória, invalidado por
`PRAGMA data_version`, que implementa as 8 ferramentas). Estado só do Rust fica em tabelas
`cg_*` no mesmo arquivo; as tabelas do CRG ficam no formato que o CRG e a extensão VS Code
leem. O SurrealDB sai por último, depois que writer, indexer, executor e servidor já usam o
SQLite.

**Tech Stack:** `rusqlite` (feature `bundled`), `petgraph`, `fs4` (flock), tokio
`spawn_blocking`.

**Spec:** `PLANO.md` (Fase 1 + decisão 1: mesmo schema do CRG, ID SHA numa coluna/tabela
extra). Referência de comportamento: `../code-review-graph` (`graph.py`, `migrations.py`,
`locking.py`) e as funções `fn::*` em `schema/codegraph_v2.surql` (linhas 279–620), que
definem a semântica das 8 ferramentas a portar.

## Global Constraints

- Banco: `<project_root>/.code-review-graph/graph.db`, `journal_mode=WAL`,
  `busy_timeout=5000`, `foreign_keys` desligado (o CRG não usa FKs).
- DDL das tabelas CRG copiado **literalmente** do dump de um banco v15 criado pelo próprio CRG
  (`nodes`, `edges`, `metadata`, `flows`, `flow_memberships`, `communities`,
  `nodes_fts_state`, `nodes_fts` FTS5 `content='nodes'`, `tokenize='porter unicode61'`, e
  todos os índices). `community_summaries`, `flow_snapshots` e `risk_index` **não** existem
  em v15 (dropados pela migração v15).
- `metadata.schema_version`: ausente → cria v15; `"15"` → abre; outro valor → erro
  `"graph.db schema version {v} is not supported (expected 15); run code-review-graph update"`.
  Não portar migrações.
- Caminhos gravados absolutos (como o CRG). `qualified_name` = `<abs_path>` para nó de
  arquivo, `<abs_path>::<symbol>` para os demais.
- Lock de escrita compatível com o CRG: `flock` exclusivo em
  `.code-review-graph/write.lock`, PID do dono escrito no arquivo.
- Nenhum `project_id` nas tabelas CRG (um banco por repositório). `project_id` continua nas
  tabelas `cg_*` só onde a API atual o recebe.
- Ferramentas mantêm nomes, parâmetros e os structs de resultado atuais
  (`DependencyNode`, `CallChainNode`, `HubNode`, …); limites e defaults iguais aos `fn::*`.
- `edge_type` de entrada aceita o nome Rust (`calls`) e o CRG (`CALLS`, `IMPORTS_FROM`).

## Mapeamentos (documentar em `docs/architecture/storage-sqlite.md`, Task 1)

| Rust `NodeType` | `nodes.kind` CRG |
|---|---|
| Function | Function |
| Class, Struct, Trait, Interface | Class |
| Enum, Type | Type |
| Module, Directory, Variable, Import, Other(s) | nome Rust (`Module`, …, `s`) — sem equivalente; Fase 2 decide |

| Rust `EdgeType` | `edges.kind` CRG |
|---|---|
| Calls | CALLS |
| Imports | IMPORTS_FROM |
| Extends | INHERITS |
| Implements | IMPLEMENTS |
| Contains | CONTAINS |
| References | REFERENCES |
| Defines, Uses, Other(s) | maiúsculas (`DEFINES`, `USES`, `S`) |

Conjunto de dependência (hubs, coupling, hotspots) = imagem de `$dependency_edge_types` do v2
por esse mapa (`calls, defines, imports, uses, extends, implements, references, depends_on,
exports, reexports, enables, generates, flows_to, returns, captures, mutates`).

Tabelas `cg_*`:

```sql
CREATE TABLE IF NOT EXISTS cg_nodes (
  uuid TEXT PRIMARY KEY, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL,
  rust_kind TEXT, complexity REAL, content TEXT, attributes TEXT NOT NULL DEFAULT '{}');
CREATE INDEX IF NOT EXISTS idx_cg_nodes_qn ON cg_nodes(qualified_name);
CREATE INDEX IF NOT EXISTS idx_cg_nodes_file ON cg_nodes(file_path);
CREATE TABLE IF NOT EXISTS cg_edges (
  uuid TEXT PRIMARY KEY, edge_id INTEGER NOT NULL, from_uuid TEXT NOT NULL,
  to_uuid TEXT NOT NULL, file_path TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_cg_edges_from ON cg_edges(from_uuid);
CREATE INDEX IF NOT EXISTS idx_cg_edges_to ON cg_edges(to_uuid);
CREATE TABLE IF NOT EXISTS cg_file_metadata (
  project_id TEXT NOT NULL, file_path TEXT NOT NULL, record TEXT NOT NULL,
  PRIMARY KEY (project_id, file_path));
CREATE TABLE IF NOT EXISTS cg_project_metadata (
  project_id TEXT PRIMARY KEY, record TEXT NOT NULL);
```

`record` = JSON serde de `FileMetadataRecord` / `ProjectMetadataRecord`.

Arestas cujo nó de origem/destino ainda não foi gravado entram com
`source_qualified`/`target_qualified` = `"cg:<uuid>"`; `resolve_pending_edges()` troca pelo
`qualified_name` real (chamado em `flush`, `reconcile_catalog` e `finalize`). Colisão de
`qualified_name` entre nós distintos (mesmo símbolo, linhas diferentes): o segundo recebe
sufixo `@L<line_start>`.

## Review Focus

1. Banco criado pelo CRG (Python) aberto pelo Rust e vice-versa: o CRG deve abrir o nosso
   sem migrar nem reclamar (`schema_version=15`, FTS consistente).
2. Dois nós Rust com o mesmo símbolo no mesmo arquivo (overload, `Import` repetido): ambos
   gravados, arestas não se misturam.
3. Aresta gravada antes dos nós (writer concorrente): nenhuma aresta perdida ou com `cg:`
   remanescente depois do `flush`.
4. Reindexar após apagar/renomear arquivo: nós, arestas (`edges` e `cg_edges`), FTS e
   `cg_file_metadata` do arquivo velho somem.
5. `codegraph start stdio` lendo enquanto `codegraph index` escreve (WAL): leitor não trava
   nem vê grafo pela metade como pronto; snapshot recarrega após o commit.

---

### Task 1: Schema CRG v15 + abertura do banco

**Files:**
- Create: `crates/codegraph-graph/src/sqlite_store/mod.rs`, `sqlite_store/schema.rs`,
  `sqlite_store/schema_v15.sql` (DDL CRG literal + `cg_*`), `docs/architecture/storage-sqlite.md`
- Create: `crates/codegraph-graph/tests/fixtures/crg_v15_empty.db` (gerado pelo CRG:
  `GraphStore(path).close()`), `crates/codegraph-graph/tests/sqlite_schema_test.rs`
- Modify: `Cargo.toml` (workspace deps `rusqlite = { version = "0.37", features = ["bundled"] }`,
  `petgraph = "0.8"`, `fs4 = "0.13"`), `crates/codegraph-graph/Cargo.toml` (feature
  `sqlite`, default on), `crates/codegraph-graph/src/lib.rs`

**Interfaces:**
- Produces: `pub struct SqliteGraphStore` (Clone; `Arc<std::sync::Mutex<rusqlite::Connection>>` +
  `root: PathBuf`); `SqliteGraphStore::open(project_root: &Path) -> Result<Self>`;
  `SqliteGraphStore::open_in_memory(project_root: &Path) -> Result<Self>` (testes);
  `pub fn db_path(project_root: &Path) -> PathBuf`; `pub const SCHEMA_VERSION: u32 = 15`.

- [ ] Teste `fresh_database_has_crg_v15_layout`: `open` em tempdir cria
      `.code-review-graph/graph.db`; `sqlite_master` contém todas as tabelas/índices CRG +
      `cg_*`; `metadata.schema_version == "15"`; `PRAGMA journal_mode == "wal"`.
- [ ] Teste `opens_database_created_by_crg`: copia o fixture para tempdir, `open` funciona e
      adiciona só `cg_*` (nenhuma tabela CRG recriada; `sqlite_master.sql` das tabelas CRG
      idêntico antes/depois).
- [ ] Teste `rejects_other_schema_versions`: `schema_version` = `"14"` e `"16"` → erro com a
      mensagem dos Global Constraints.
- [ ] Rodar: `cargo test -p codegraph-graph --test sqlite_schema_test` → FAIL.
- [ ] Implementar `open`: cria diretório, aplica pragmas, `execute_batch(schema_v15.sql)`
      dentro de uma transação só se `metadata` não existe; senão valida versão e aplica só o
      bloco `cg_*`.
- [ ] Rodar o teste → PASS. Escrever `docs/architecture/storage-sqlite.md` com as tabelas de
      mapeamento acima.
- [ ] Commit `feat(graph): open CRG v15 SQLite store` + push na `main`.

### Task 2: Escrita de nós, arestas e metadados

**Files:**
- Create: `crates/codegraph-graph/src/sqlite_store/write.rs`, `sqlite_store/mapping.rs`
- Create: `crates/codegraph-graph/src/records.rs` (move `FileMetadataRecord`,
  `ProjectMetadataRecord` para fora do `#[cfg(feature = "surrealdb")]`; `surrealdb_storage`
  passa a reexportar)
- Test: `crates/codegraph-graph/tests/sqlite_write_test.rs`

**Interfaces:**
- Consumes: `SqliteGraphStore` (Task 1).
- Produces (todos `async`, corpo em `spawn_blocking`, uma transação por chamada):
  `upsert_nodes_batch(&self, &[CodeNode]) -> Result<()>`,
  `upsert_edges_batch(&self, &[CodeEdge]) -> Result<()>`,
  `upsert_file_metadata_batch(&self, &[FileMetadataRecord]) -> Result<()>`,
  `upsert_project_metadata(&self, ProjectMetadataRecord) -> Result<()>`,
  `delete_data_for_files(&self, project_id: &str, &[String]) -> Result<()>`,
  `resolve_pending_edges(&self) -> Result<usize>`;
  em `mapping.rs`: `crg_node_kind(&NodeType) -> String`, `crg_edge_kind(&EdgeType) -> String`,
  `edge_kind_from_input(&str) -> String`, `qualified_name(root, &CodeNode) -> String`,
  `DEPENDENCY_EDGE_KINDS: &[&str]`.

Regras de `upsert_nodes_batch`: `name`, `file_path` absoluto, `line_start = location.line`,
`line_end = location.end_line`, `language`, `parent_name`/`params`/`return_type` de
`metadata.attributes` quando existirem, `is_test` = `attributes["is_test"] == "true"`,
`symbol` = parte após `::`, `name_tokens` = split camelCase/snake_case de `name`,
`updated_at` = epoch f64, `extra = '{}'`; `cg_nodes` com uuid, `rust_kind`, `complexity`,
`content`, `attributes` JSON. Upsert por `uuid`: se o uuid já existe, atualiza a linha de
`nodes` com o mesmo `qualified_name`.

- [ ] Testes (FAIL primeiro): `node_round_trip_uses_crg_columns` (Python `Service.run` →
      `kind=Function`, `qualified_name=<abs>::Service.run`, `cg_nodes.uuid` = id);
      `duplicate_symbol_gets_line_suffix` (Review Focus 2);
      `edge_before_nodes_is_resolved_on_flush` (Review Focus 3: aresta gravada antes, `cg:`
      some após `resolve_pending_edges`, `kind=CALLS`);
      `edge_upsert_is_idempotent` (mesma aresta 2× → 1 linha em `edges`);
      `delete_files_removes_nodes_edges_metadata` (Review Focus 4, incluindo `cg_edges` e
      arestas de entrada vindas de outros arquivos);
      `edge_kind_mapping_round_trips` (tabela de mapeamento + `edge_kind_from_input("imports")
      == "IMPORTS_FROM"`, `("IMPORTS_FROM") == "IMPORTS_FROM"`).
- [ ] Implementar → `cargo test -p codegraph-graph --test sqlite_write_test` PASS.
- [ ] Commit `feat(graph): write CodeNode/CodeEdge into CRG tables` + push.

### Task 3: Reconciliação, marcador de pronto e lock

**Files:**
- Create: `crates/codegraph-graph/src/sqlite_store/reconcile.rs`, `sqlite_store/lock.rs`
- Test: `crates/codegraph-graph/tests/sqlite_reconcile_test.rs`

**Interfaces:**
- Produces: `reconcile_catalog(&self, project: &str, nodes: Vec<String>, edges: Vec<String>,
  files: Vec<String>) -> Result<()>` (mesma semântica do SurrealDB, sem `chunks`; usa tabelas
  temporárias); `get_file_metadata_for_project(&self, &str) -> Result<Vec<FileMetadataRecord>>`;
  `get_file_metadata(&self, file_path: &str) -> Result<Option<FileMetadataRecord>>`;
  `project_marker(&self, &str) -> Result<Option<ProjectMetadataRecord>>` (precisa de
  `Deserialize` em `ProjectMetadataRecord`); `delete_project_marker(&self, &str)`;
  `counts(&self) -> Result<GraphCounts { nodes, edges, files }>`;
  `finalize(&self, build_type: &str) -> Result<()>` (resolve arestas pendentes, reconstrói
  `nodes_fts` com `'rebuild'` e espelha `nodes_fts_state`, grava `metadata`
  `last_updated` ISO-8601 sem fuso, `last_build_type`, `repo_root`, `build_state=complete`,
  `fts_state_synced=2`);
  `WriteLock::acquire(project_root: &Path, blocking: bool) -> Result<Option<WriteLock>>`
  (`None` = outro processo segura; Drop libera).

- [ ] Testes (FAIL primeiro): `reconcile_removes_ids_outside_catalog`;
      `finalize_leaves_fts_searchable` (`SELECT … FROM nodes_fts WHERE nodes_fts MATCH 'service'`
      acha o nó); `marker_round_trip_and_delete`; `second_writer_lock_is_refused`
      (dois `acquire(false)` → segundo `None`; `flock -n` externo também vê o lock).
- [ ] Teste opcional `crg_reads_our_database` (gate `CODEGRAPH_RUN_CRG_COMPAT=1`): grava
      fixture, roda `uv run --project ../code-review-graph code-review-graph status --repo …`
      e espera exit 0 e contagens iguais (Review Focus 1).
- [ ] Implementar → PASS. Commit `feat(graph): catalog reconciliation, FTS finalize, write lock`
      + push.

### Task 4: Camada de consulta petgraph (8 ferramentas)

**Files:**
- Create: `crates/codegraph-graph/src/graph_query/mod.rs` (`GraphQuery`),
  `graph_query/snapshot.rs`, `graph_query/types.rs` (move os structs de resultado de
  `graph_functions.rs` para cá, sem `cfg`)
- Test: `crates/codegraph-graph/tests/graph_query_test.rs` (fixture: grafo pequeno gravado
  via Task 2, com ciclo A↔B, cadeia de chamadas de 3 níveis, um hub com 6 dependentes, uma
  função de complexidade 12)

**Interfaces:**
- Consumes: `SqliteGraphStore`, `mapping::{edge_kind_from_input, DEPENDENCY_EDGE_KINDS}`.
- Produces: `GraphQuery::new(store: SqliteGraphStore) -> Self` (Clone) com métodos de mesma
  assinatura que `GraphFunctions`: `find_nodes_by_name(&str, usize)`,
  `get_transitive_dependencies(&str, &str, i32)`, `get_reverse_dependencies(&str, &str, i32)`,
  `detect_circular_dependencies(&str)`, `trace_call_chain(&str, i32)`,
  `calculate_coupling_metrics(&str)`, `get_hub_nodes(i32)`,
  `get_complexity_hotspots(f32, i32)`, `count_nodes_for_project()`. Node id de saída = uuid
  (`cg_nodes`) ou `qualified_name` quando o nó veio do CRG; entrada aceita uuid,
  `nodes:\`uuid\``, `nodes:⟨uuid⟩` e `qualified_name`.

Semântica = `fn::*` do v2: depth fora de 1..=10 → 3 (call chain → 5); BFS registra a
primeira profundidade; call chain inclui `call_path` e até 10 `called_by`; ciclo = par
recíproco `from < to`; hubs: `min_degree <= 0` → 5, top 50 por grau total; hotspots:
`kind = Function`, `complexity >= min` (default 5), `limit` 1..=100 (default 20),
`risk = complexity × (afferent + 1)`, instability arredondada a 4 casas; coupling com
categorias `stable < 0.3 < balanced < 0.7 < unstable`, instability a 6 casas.
Snapshot: `StableDiGraph` carregado de `nodes ⟕ cg_nodes` + `edges`, guardado em
`RwLock<Option<(i64, Arc<Snapshot>)>>`, recarregado quando `PRAGMA data_version` muda.

- [ ] Um teste por ferramenta com os valores do fixture (FAIL primeiro), mais
      `snapshot_reloads_after_write` (Review Focus 5: segunda conexão escreve, próxima
      consulta vê o dado) e `accepts_legacy_record_id_forms`.
- [ ] Implementar → `cargo test -p codegraph-graph --test graph_query_test` PASS.
- [ ] Commit `feat(graph): petgraph query layer for graph tools` + push.

### Task 5: Executor, servidor MCP e CLI sobre SQLite

**Files:**
- Modify: `crates/codegraph-mcp-tools/src/graph_tool_executor.rs` (`Arc<GraphFunctions>` →
  `GraphQuery`), `crates/codegraph-mcp-tools/Cargo.toml`,
  `crates/codegraph-mcp-server/src/official_server.rs:233-250` (`open_executor` abre
  `SqliteGraphStore::open(project_root)`), `src/tool_cli.rs`, `src/bin/codegraph.rs`
  (`db-check` → abre o SQLite e imprime `counts()`).
- Test: os existentes (`every_graph_tool_is_exposed_over_mcp`, `tool_cli_integration`,
  `e2e_mcp_tools`) ajustados ao novo store.

- [ ] Ajustar testes para criar o fixture via `SqliteGraphStore` → FAIL.
- [ ] Trocar a dependência → `cargo test -p codegraph-mcp-tools -p codegraph-mcp-server
      --all-features` e `cargo test -p codegraph-mcp --test e2e_mcp_tools` PASS.
- [ ] Commit `feat(server): serve graph tools from SQLite` + push.

### Task 6: Writer e indexer sobre SQLite

**Files:**
- Modify: `crates/codegraph-mcp/src/writer.rs` (`SurrealDbStorage` → `SqliteGraphStore`;
  remove `Job::{NodeEmbeddings,SymbolEmbeddings,ChunkEmbeddings}`; pool de 1 conexão de
  escrita), `crates/codegraph-mcp/src/indexer.rs` (substituir cada query crua: linhas ~590
  marcador → `project_marker`; ~1235 → `delete_project_marker`; ~1601 →
  `reconcile_catalog` sem chunks; ~1896/1918/2229/2282/2319/2345/2371 → `counts`,
  `get_file_metadata`, `project_marker`; ~368 e ~1627 `vector_indexes` removidos; adquirir
  `WriteLock` no início de `index_project` e chamar `finalize("full"|"incremental")` antes
  do marcador de pronto), `crates/codegraph-mcp-daemon/src/daemon/*`,
  `crates/codegraph-mcp/tests/{reconciliation_test,daemon_watch}.rs`.

- [ ] Rodar `cargo test -p codegraph-mcp --test reconciliation_test` com o store novo → FAIL
      de compilação, depois ajustar até PASS; mesma coisa para `daemon_watch` e
      `index_cli_integration`.
- [ ] Verificação manual: `codegraph index <fixture python> -r` cria
      `.code-review-graph/graph.db`; `codegraph tool run find_nodes_by_name --params
      '{"name":"Service"}'` acha o nó; `sqlite3 graph.db "select count(*) from nodes"` > 0.
- [ ] Commit `feat(indexer): persist the graph in SQLite` + push.

### Task 7: Remoção do SurrealDB

**Files:**
- Delete: `crates/codegraph-graph/src/{surrealdb_storage,surrealdb_migrations,graph_functions,vector_indexes}.rs`,
  `src/bin/surreal_smoke_test.rs`, `schema/`, testes `schema_*`, `embedded_store_test`,
  `ingestion_write_test`, `graph_functions_live_probe_test`,
  `semantic_search_nodes_via_chunks_test`, `codegraph-core/src/integration/graph_vector.rs`
  (+ uso em `incremental/updater.rs`).
- Modify: Cargos (sem `surrealdb`, `reqwest`/`native-tls` onde só serviam a ele), config
  (`CODEGRAPH_SURREALDB_*`, `[embedding]`, `[llm]`, `[rerank]`, `--batch-size`,
  `estimate`), `CLAUDE.md`, `AGENTS.md`, `PLANO.md`.

- [ ] `cargo tree -p codegraph-mcp-server --all-features -i openssl-sys` → nenhum resultado.
- [ ] `cargo build --release -p codegraph-mcp-server --bin codegraph --features full` e
      `ldd target/release/codegraph | grep -c ssl` → `0`.
- [ ] Suíte completa (`cargo test --workspace --all-features --no-fail-fast` pulando os 5
      watchers WSL) verde.
- [ ] Commit `chore: drop SurrealDB` + push; marcar Fase 1 ✅ no `PLANO.md`.
