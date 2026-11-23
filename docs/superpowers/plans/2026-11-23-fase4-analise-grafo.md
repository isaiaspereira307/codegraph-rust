# Fase 4 — Análise de grafo: plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** portar do CRG (v2.4.0) os flows de execução, as comunidades, os insights de grafo e o
risk score, gravando `flows`/`flow_memberships`/`communities`/`nodes.community_id` no mesmo
`graph.db` v15 e expondo as ferramentas MCP equivalentes.

**Architecture:** um módulo novo `codegraph-graph/src/analysis/` carrega um `AnalysisGraph`
(nós e arestas crus do SQLite, ordem por `id`) e calcula flows e comunidades num pós-processo
chamado ao fim do `finalize`, dentro do lock de escrita. As ferramentas de leitura (flows,
comunidades, insights, risco) leem as tabelas e o `AnalysisGraph`; nada depende de `cg_*`, então
funcionam também num banco escrito pelo CRG. Leiden é implementado à mão sobre listas de
adjacência, determinístico (sem RNG).

**Tech Stack:** Rust, `rusqlite`, `petgraph` 0.8 (`tarjan_scc` para ciclos de import), `regex`
(workspace), rmcp.

**Spec:** `PLANO.md` (Fase 4) + comportamento do CRG em `../code-review-graph/code_review_graph/`:
`flows.py`, `communities.py`, `analysis.py`, `changes.py:1275-1454` (risco),
`tools/flows_tools.py`, `tools/community_tools.py`, `tools/analysis_tools.py`,
`tools/review.py:115-245,875`. Os números de linha citados abaixo são desses arquivos.

## Global Constraints

- Não editar `schema_v15.sql`. Não recriar `risk_index`, `flow_snapshots` nem
  `community_summaries` (o v15 do CRG os removeu; risco é calculado sob demanda).
- Sem dependência nova fora do workspace. `regex` entra em `codegraph-graph` como
  `regex = { workspace = true }`.
- Saída determinística: mesma entrada → mesmas linhas no banco e mesmo JSON. Todo iterador sobre
  `HashMap`/`HashSet` que afeta saída é ordenado antes. Nada de RNG.
- Ordem "do SQLite" no CRG (rowid) vira `ORDER BY id` explícito aqui.
- Arredondamento: o CRG usa `round()` do Python (meio-par). Usar um helper
  `round_half_even(x, places)` em `analysis/mod.rs` sempre que o spec diz `round`.
- `_sanitize_name` do CRG (graph.py:5865): remove `[\x00-\x08\x0b-\x1f]` e corta em 256 chars.
  Helper `sanitize_name` em `analysis/mod.rs`.
- Arquivos novos começam com duas linhas `// ABOUTME:`.
- Ferramenta nova = schema em `graph_tool_schemas.rs` + braço e `execute_*` em
  `graph_tool_executor.rs` + `#[tool]` em `official_server.rs` (o teste
  `every_graph_tool_is_exposed_over_mcp` cobra o último).
- Formato das respostas das ferramentas: o mesmo envelope já usado pelo executor
  (`{"tool", "parameters", "result"}`); dentro de `result`, os nomes de campo do CRG.
- Constante compartilhada `SECURITY_KEYWORDS` (constants.py:112):
  `auth, login, password, token, session, crypt, secret, credential, permission, sql, query,
  execute, connect, socket, request, http, sanitize, validate, encrypt, decrypt, hash, sign,
  verify, admin, privilege`.
- Testes rodam com `cargo test -p codegraph-graph` e `cargo test -p codegraph-mcp-tools`; as
  tasks de ferramenta também `cargo test -p codegraph-mcp-server --lib`.

## Decisões tomadas (defaults; revisar)

1. **Flows sempre recalculados por inteiro** em cada `finalize` (full ou incremental). O
   `incremental_trace_flows` do CRG existe por desempenho; só portar se a Task 3 medir mais de
   2 s neste repositório. Consequência: `flows.id` muda a cada build (o CRG preserva ids de flows
   não afetados). Ferramentas aceitam `flow_name` além de `flow_id`.
2. **Comunidades também recalculadas por inteiro** (o incremental do CRG cai no full sempre que
   um arquivo alterado tem comunidade, ou seja, quase sempre).
3. **Leiden determinístico, não igual ao igraph.** Paridade de partição com o CRG é impossível
   (RNG diferente). Critério: mesmas propriedades (comunidades conexas, modularidade), e a
   concordância com o CRG medida no harness (ARI) e registrada, sem limiar de aceite.
   `CRG_LEIDEN_SEED` é ignorado. `metadata.community_detector` grava `leiden` (ou `file`,
   `file+leiden`), como o CRG, para quem lê o banco.
4. **Betweenness**: Brandes exato até 5000 nós; acima disso, amostra de `k = 500` fontes
   escolhidas deterministicamente (nós ordenados por `qualified_name`, passo `n / k`), com
   reescala `n / k`. O CRG amostra sem seed.
5. **Pós-processo**: nível `full` (padrão), `minimal` (sem flows nem comunidades) ou `none`, como
   o CRG. CLI: `--skip-flows` → `minimal`, `--skip-postprocess` → `none` em `build`, `update` e
   `index`. Variável `CODEGRAPH_POSTPROCESS=full|minimal|none` para o watcher e o MCP.
6. **Decorators**: o writer passa a gravar `nodes.extra = {"decorators": [...]}` quando o nó
   tem o atributo `decorators` (hoje só em `cg_nodes.attributes`, separado por vírgula), para o
   CRG ler o nosso banco do mesmo jeito. O `AnalysisGraph` lê só `nodes.extra`.
7. **`is_test_file`**: o Rust já marca `is_test=1` em todo nó de arquivo de teste (Fase 2); a
   detecção de entry points usa só `nodes.is_test` e `kind == 'Test'`.
8. **Churn** no risco: opcional (`include_churn`, padrão `false` em `detect_changes`, `true` em
   `get_review_context` como o CRG), contagem de commits por arquivo em 90 dias via `git log`.

## Review Focus

1. Banco escrito pelo CRG (sem `cg_nodes`, `extra` com outros campos): toda ferramenta da Fase 4
   funciona e devolve dados, não erro. Teste na Task 1 e na Task 9.
2. Grafo sem nenhuma aresta CALLS (repo só com dados/configs) ou banco vazio: `list_flows`,
   `list_communities`, insights e risco devolvem listas vazias e `total: 0`. Testes nas Tasks 3,
   7 e 10.
3. `finalize` repetido sem mudança de fonte: mesmo número de linhas em `flows`,
   `flow_memberships`, `communities`, e o mesmo `community_id` por nó. Testes nas Tasks 3 e 8.
4. Recursão e ciclos de chamada (`a→b→a`, `a→a`): BFS termina, `a` aparece uma vez no `path`.
   Teste na Task 3.
5. Alvos de texto não resolvidos (`target_qualified` sem nó): não viram nó em comunidades nem
   betweenness, mas contam em `external_count` da criticidade. Testes nas Tasks 3 e 6.

---

## Estrutura de arquivos

```
crates/codegraph-graph/src/analysis/
  mod.rs            AnalysisGraph, helpers (round_half_even, sanitize_name), SECURITY_KEYWORDS,
                    PostprocessLevel, run_postprocess
  flows/entry.rs    detect_entry_points (padrões de decorator e de nome)
  flows/mod.rs      trace_flows, compute_criticality, store_flows
  communities/leiden.rs   WeightedGraph + leiden() puro
  communities/mod.rs      detect_communities, file-based, split_oversized, cohesion, store
  communities/naming.rs   nomes e dedupe
  insights.rs       hubs, bridges (Brandes), knowledge gaps, surprising, suggested questions
  risk.rs           compute_risk_score e sinais
crates/codegraph-graph/src/graph_query/analysis_queries.rs
                    métodos GraphQuery: list_flows, get_flow, affected_flows, list_communities,
                    get_community, architecture_overview, insights, risk_scores
crates/codegraph-graph/tests/analysis_*_test.rs   fixtures SQLite montadas à mão
crates/codegraph-mcp-tools/src/{graph_tool_schemas,graph_tool_executor,review}.rs
crates/codegraph-mcp-server/src/official_server.rs, src/bin/codegraph.rs
scripts/parity/parity.py   comparação de flows e comunidades
```

Fixture comum de teste: helper `tests/common/analysis_fixture.rs` com
`fn fixture_db(nodes: &[(&str kind, &str qn, &str name, &str file, bool is_test)],
edges: &[(&str kind, &str src, &str tgt)]) -> (TempDir, SqliteGraphStore)`, que abre um store
novo e insere direto em `nodes`/`edges` por SQL (sem `cg_nodes`, cobrindo o Review Focus 1).

---

### Task 1: `AnalysisGraph` e decorators em `nodes.extra`

**Files:**
- Create: `crates/codegraph-graph/src/analysis/mod.rs`
- Modify: `crates/codegraph-graph/src/lib.rs` (`pub mod analysis` sob `feature = "sqlite"`),
  `crates/codegraph-graph/Cargo.toml` (regex), `crates/codegraph-graph/src/sqlite_store/write.rs:166-171`
- Test: `crates/codegraph-graph/tests/analysis_graph_test.rs`, `tests/common/analysis_fixture.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct AnalysisNode { pub id: i64, pub qualified: String, pub name: String,
      pub kind: String, pub file_path: String, pub language: Option<String>,
      pub line_start: Option<i64>, pub line_end: Option<i64>, pub is_test: bool,
      pub decorators: Vec<String>, pub community_id: Option<i64> }
  pub struct AnalysisEdge { pub kind: String, pub source: String, pub target: String,
      pub file_path: String, pub line: i64 }
  pub struct AnalysisGraph { pub nodes: Vec<AnalysisNode>, pub edges: Vec<AnalysisEdge>, .. }
  impl AnalysisGraph {
      pub fn load(conn: &rusqlite::Connection) -> anyhow::Result<Self>; // ORDER BY id
      pub fn index_of(&self, qualified: &str) -> Option<usize>;
      pub fn by_id(&self, id: i64) -> Option<usize>;
  }
  pub fn round_half_even(x: f64, places: i32) -> f64;
  pub fn sanitize_name(s: &str) -> String;
  pub const SECURITY_KEYWORDS: &[&str];
  ```

- [ ] **Step 1: testes que falham**
  - `load_reads_nodes_and_edges_in_id_order`: fixture com 3 nós e 2 arestas; `nodes[i].id`
    crescente, `edges` na ordem de inserção, `index_of("f.py::a") == Some(0)`.
  - `decorators_come_from_extra_as_list_or_string`: `extra = '{"decorators":["app.get"]}'` →
    `["app.get"]`; `extra = '{"decorators":"click.command"}'` → `["click.command"]`;
    `extra = '{}'` ou JSON inválido → `[]`.
  - `round_half_even(0.12345, 4) == 0.1234`, `round_half_even(0.12355, 4) == 0.1236`.
  - `sanitize_name("a\u{1}b") == "ab"`; string de 300 chars → 256.
  - Em `sqlite_write_test.rs`: `writer_stores_decorators_in_extra` — nó com atributo
    `decorators = "app.get,login_required"` grava `extra = {"decorators":["app.get","login_required"]}`;
    sem atributo, `extra = '{}'`.
- [ ] **Step 2:** `cargo test -p codegraph-graph --test analysis_graph_test` → falha (módulo não existe).
- [ ] **Step 3:** implementar. `load` faz duas consultas: `SELECT id, qualified_name, name, kind,
  file_path, language, line_start, line_end, is_test, extra, community_id FROM nodes ORDER BY id`
  e `SELECT kind, source_qualified, target_qualified, file_path, line FROM edges ORDER BY id`.
  No writer, trocar o literal `'{}'` por um parâmetro: `{"decorators":[..]}` (split por `,`,
  trim, vazios fora) ou `{}`.
- [ ] **Step 4:** `cargo test -p codegraph-graph` verde.
- [ ] **Step 5:** commit `feat(graph): add AnalysisGraph loader and store decorators in nodes.extra`.

### Task 2: detecção de entry points

**Files:**
- Create: `crates/codegraph-graph/src/analysis/flows/entry.rs`, `flows/mod.rs` (só `mod entry; pub use`)
- Test: `crates/codegraph-graph/tests/analysis_flows_test.rs`

**Interfaces:**
- Consumes: `AnalysisGraph` (Task 1).
- Produces: `pub fn detect_entry_points(g: &AnalysisGraph, include_tests: bool) -> Vec<usize>`
  (índices em `g.nodes`, ordem de `id`, sem repetição).

Regras (flows.py:496-560), na ordem:
1. Candidatos: `kind ∈ {Function, Test}`.
2. Se `!include_tests`: pula `is_test` ou `kind == "Test"`.
3. Conjunto "chamado": `target` de toda aresta `CALLS` cuja origem não é nó `File` (origem sem nó
   conta). Para nós com `language ∈ {ruby, java}` usar o conjunto "chamado fora de teste" (origem
   sem nó, ou origem não-File com `is_test = false`).
4. Entry se: `qualified ∉ chamado` **ou** algum decorator casa um padrão de framework **ou** o
   `name` casa um padrão de nome.
5. Padrões de decorator e de nome: copiar literalmente de flows.py:346-397 (decorators, com as
   flags de caixa indicadas no fonte), flows.py:406-445 (nomes) e flows.py:448 (PHP:
   `^(boot|register)$`, `^__invoke$`, só quando `language == "php"`). Compilar uma vez com
   `std::sync::LazyLock<Vec<Regex>>`; casar com `is_match` (= `re.search`). `(?i)` onde o CRG usa
   `re.IGNORECASE`; o padrão JUnit usa `(?s)`.
6. Ignorar `extra.verilog_kind` (o Rust não indexa Verilog).

- [ ] **Step 1: testes que falham**
  - `uncalled_function_is_entry`: `a` chama `b`; entries = `[a]`.
  - `called_function_with_route_decorator_is_entry`: `b` chamado, decorator `app.get` → entry.
  - `called_function_named_main_is_entry`: `main` chamado → entry; `mainly` chamado → não.
  - `tests_skipped_unless_included`: nó `Test` não chamado; `include_tests=false` → fora; `true` → dentro.
  - `call_from_file_node_does_not_count_as_called`: só `File → b` chama `b` → `b` é entry.
  - `java_called_only_from_tests_is_entry`: `b` (java) chamado só por nó com `is_test` → entry.
  - `php_boot_only_for_php`: `boot` chamado, python → não; php → sim.
- [ ] **Step 2:** rodar `cargo test -p codegraph-graph --test analysis_flows_test` → falha.
- [ ] **Step 3:** implementar.
- [ ] **Step 4:** verde.
- [ ] **Step 5:** commit `feat(graph): detect flow entry points like code-review-graph`.

### Task 3: trace, criticidade, gravação e pós-processo no `finalize`

**Files:**
- Modify: `crates/codegraph-graph/src/analysis/flows/mod.rs`, `analysis/mod.rs`,
  `crates/codegraph-graph/src/sqlite_store/reconcile.rs:146` (`finalize`),
  `crates/codegraph-mcp/src/indexer.rs:1449` e `:1720`, `crates/codegraph-mcp/src/reconciliation.rs:49`,
  `crates/codegraph-mcp-server/src/bin/codegraph.rs` (flags)
- Test: `crates/codegraph-graph/tests/analysis_flows_test.rs`, `crates/codegraph-mcp/tests/reconciliation_test.rs`

**Interfaces:**
- Consumes: `detect_entry_points` (Task 2).
- Produces:
  ```rust
  pub struct Flow { pub name: String, pub entry_point: String, pub entry_point_id: i64,
      pub path: Vec<i64>, pub depth: u32, pub node_count: usize, pub file_count: usize,
      pub criticality: f64 }
  pub const FLOW_MAX_DEPTH: u32 = 15;
  pub fn trace_flows(g: &AnalysisGraph, max_depth: u32) -> Vec<Flow>;
  pub fn store_flows(tx: &rusqlite::Transaction, flows: &[Flow]) -> anyhow::Result<usize>;
  #[derive(Clone, Copy, Default)] pub enum PostprocessLevel { #[default] Full, Minimal, None }
  impl std::str::FromStr for PostprocessLevel; // "full" | "minimal" | "none"
  pub fn run_postprocess(conn: &mut rusqlite::Connection, level: PostprocessLevel)
      -> anyhow::Result<PostprocessStats>; // { flows: usize, communities: usize }
  // SqliteGraphStore::finalize(&self, build_type: &str, level: PostprocessLevel)
  ```

Trace (flows.py:568-662): adjacência `calls_out[src]` com os alvos de `CALLS` em ordem de `id`,
**com repetição**; `has_tested_by` = origens de `TESTED_BY`. BFS a partir do entry
(`visited = {ep}`): ao tirar `(qn, d)`, `depth = max(depth, d)`; se `d >= max_depth` não expande;
para cada alvo não visitado **que é nó** (qualquer kind): marca, empilha `(t, d+1)`, adiciona o
`id` ao `path`. `path.len() < 2` → sem flow. `name = sanitize_name(ep.name)`. Ordenação final:
estável por `criticality` desc sobre a ordem dos entries.

Criticidade (flows.py:670-739), sobre os nós do `path`:
```
file_spread    = if files > 1 { min((files - 1) / 4, 1) } else { 0 }
external_score = min(#alvos de calls_out[n] (com repetição, todos os n do path) que não são nó / 5, 1)
security_score = min(#nós cujo name ou qualified (lowercase) contém algum SECURITY_KEYWORD / len, 1)
test_gap       = 1 - #nós em has_tested_by / len
depth_score    = min(depth / 10, 1)
crit = 0.30·file_spread + 0.20·external + 0.25·security + 0.15·test_gap + 0.10·depth
round_half_even(clamp(crit, 0, 1), 4)
```

`store_flows` (flows.py:747-805), numa transação: `DELETE FROM flow_memberships`,
`DELETE FROM flows`, `INSERT INTO flows (name, entry_point_id, depth, node_count, file_count,
criticality, path_json)` com `path_json` no formato do `json.dumps` (`"[1, 2, 3]"`, espaço após
vírgula), `INSERT OR IGNORE INTO flow_memberships (flow_id, node_id, position)` com a posição no
path, e `DELETE FROM metadata WHERE key = 'csharp_flows_dirty'`.

`run_postprocess`: `None` não faz nada; `Minimal` não faz nada nesta fase (reservado: no CRG faz
só o FTS, que o `finalize` já faz); `Full` carrega o `AnalysisGraph` depois do commit do
`finalize`, grava flows (e, a partir da Task 8, comunidades) numa segunda transação
`BEGIN IMMEDIATE`. Medir o tempo; registrar em `tracing::info!` como `flows_s`.

CLI: `build`, `update` e `index` ganham `--skip-flows` (→ `Minimal`) e `--skip-postprocess`
(→ `None`); sem flag, lê `CODEGRAPH_POSTPROCESS` (padrão `full`). Valor inválido = erro.

- [ ] **Step 1: testes que falham** (`analysis_flows_test.rs`)
  - `linear_chain_traced_bfs`: `main→a→b`, `a→c` → path `[main, a, b, c]` (ids), `depth 2`,
    `node_count 4`.
  - `recursion_terminates`: `main→a`, `a→a`, `a→main` → path `[main, a]`.
  - `depth_capped`: cadeia de 20 → `node_count 16`, `depth 15`.
  - `single_node_entry_has_no_flow`: `main` sem chamadas → nenhum flow.
  - `criticality_formula`: cadeia `main→login_user→c` (depth 2) em 2 arquivos, `c` com
    `TESTED_BY`, 2 alvos de texto → `0.30·0.25 + 0.20·0.4 + 0.25·(1/3) + 0.15·(2/3) + 0.10·0.2`
    arredondado = `0.3583`.
  - `store_flows_writes_python_json_path`: `path_json == "[1, 2]"`, memberships com posição 0,1.
  - `no_calls_no_flows` (Review Focus 2).
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar trace, criticidade, store e `run_postprocess`; trocar a assinatura
  de `finalize` e os 3 chamadores; flags na CLI.
- [ ] **Step 4:** em `reconciliation_test.rs`, `finalize_twice_keeps_flow_counts` (Review Focus 3):
  indexar o fixture Python do corpus, contar `flows`/`flow_memberships`, rodar `update` sem
  mudança, contar de novo: iguais e > 0. `skip_postprocess_leaves_flows_empty`.
- [ ] **Step 5:** `cargo test -p codegraph-graph && cargo test -p codegraph-mcp --test reconciliation_test`
  verdes; `target/debug/codegraph build .` neste repo e anotar `flows_s` no commit. Se > 2 s,
  registrar no PLANO como motivo para portar o incremental (Decisão 1).
- [ ] **Step 6:** commit `feat(graph): trace execution flows with criticality during finalize`.

### Task 4: ferramentas `list_flows`, `get_flow`, `get_affected_flows`

**Files:**
- Create: `crates/codegraph-graph/src/graph_query/analysis_queries.rs`
- Modify: `graph_query/mod.rs`, `codegraph-mcp-tools/src/graph_tool_schemas.rs`,
  `graph_tool_executor.rs`, `codegraph-mcp-server/src/official_server.rs`
- Test: `crates/codegraph-graph/tests/analysis_flows_test.rs`, testes do executor

**Interfaces:**
- Consumes: tabelas `flows`/`flow_memberships` (Task 3); `changed_files` de `review.rs` (Fase 3).
- Produces (em `GraphQuery`):
  ```rust
  pub async fn list_flows(&self, sort_by: &str, limit: usize, kind: Option<&str>)
      -> Result<(Vec<FlowRow>, usize /*total*/)>;
  pub async fn get_flow(&self, id: Option<i64>, name: Option<&str>, max_steps: usize)
      -> Result<Option<FlowDetail>>;
  pub async fn affected_flows(&self, files: &[String]) -> Result<Vec<FlowDetail>>;
  // FlowRow: id, name, entry_point_id, depth, node_count, file_count, criticality, path,
  //          created_at, updated_at
  // FlowDetail: FlowRow + steps: Vec<FlowStep{node_id, name, kind, file, line_start,
  //             line_end, qualified_name}> + total_steps + truncated
  ```

Comportamento (flows.py:997-1124, flows_tools.py, review.py:875):
- `list_flows`: `sort_by ∈ {criticality, depth, node_count, file_count, name}` (outro →
  `criticality`); `name` asc, resto desc, desempate por `id` asc. `kind` filtra pelo kind do nó
  do entry. Tool: `sort_by="criticality"`, `limit=50` (teto 200), `kind`, `detail_level`
  (`minimal` → `{name, criticality, node_count}`). Resultado `{summary: "Found N execution
  flow(s)[, showing N of M]", flows, total, truncated}`.
- `get_flow`: `flow_id` tem precedência; senão `flow_name` por substring sem caixa nos 500 de
  maior criticidade, primeiro que casa. Tool: `flow_id`, `flow_name`, `include_source=false`,
  `max_steps=50` (teto 200), `max_source_lines=400` (teto 2000, orçamento total; cada passo
  ganha `source` com linhas `"{n}: {linha}"`, reusar o leitor de snippet de `review.rs`; acabou o
  orçamento → `truncated` e `source_truncated`). Não achou → `{status: "not_found", summary:
  "No flow found matching the given criteria."}`.
- `affected_flows`: arquivos → nós (`file_path` igual, depois de `canonical_file_key`) → flows
  distintos, ordenados por criticidade desc, desempate por `id`. Tool `get_affected_flows`:
  `changed_files` (padrão: `git diff` contra `base_ref="HEAD~1"`, reusar `changed_files` de
  `review.rs`), `detail_level`, `max_flows=50`; teto 25 em `standard`, 500 em `minimal`;
  `minimal` → `{id, name, criticality, depth, node_count, file_count}`; `standard` divide um
  orçamento de 400 passos entre os flows (`steps_omitted: true` no que foi cortado). Sem
  arquivos → `{summary: "No changed files detected.", affected_flows: [], total: 0}`.

- [ ] **Step 1: testes que falham**: `list_flows_sorts_and_caps`, `list_flows_filters_by_entry_kind`,
  `get_flow_by_name_substring_case_insensitive`, `get_flow_missing_is_none`,
  `affected_flows_from_file`, `step_budget_marks_steps_omitted` (executor), e o teste de
  exposição MCP pegando as 3 tools novas.
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar queries, schemas, executor, `#[tool]`.
- [ ] **Step 4:** `cargo test -p codegraph-graph && cargo test -p codegraph-mcp-tools && cargo test -p codegraph-mcp-server --lib` verdes.
- [ ] **Step 5:** commit `feat(mcp): add list_flows, get_flow and get_affected_flows`.

### Task 5: Leiden determinístico

**Files:**
- Create: `crates/codegraph-graph/src/analysis/communities/leiden.rs`, `communities/mod.rs` (mods)
- Test: unitários no próprio `leiden.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct WeightedGraph { /* n nós, adjacência simétrica sem laços */ }
  impl WeightedGraph {
      pub fn new(n: usize) -> Self;
      pub fn add_edge(&mut self, a: usize, b: usize, w: f64); // a != b; ignora par repetido
  }
  /// Partição que maximiza modularidade com resolução; ids de comunidade 0..k em ordem do
  /// primeiro nó de cada uma.
  pub fn leiden(g: &WeightedGraph, resolution: f64, iterations: u32) -> Vec<usize>;
  ```

Algoritmo (Traag et al. 2019, objetivo modularidade `Q = 1/2m Σ (A_ij − γ k_i k_j / 2m) δ(c_i,c_j)`):
1. Partição inicial: cada nó sozinho (ou a da iteração anterior).
2. **Movimento local** com fila: começa com todos os nós em ordem de índice; tira um nó, calcula
   o ganho `ΔQ(i→C) = k_{i,C} − γ·k_i·Σ_tot(C)/(2m)` para cada comunidade vizinha (e a própria,
   após removê-lo); move para a de maior ganho estritamente positivo sobre ficar; empate → menor
   id de comunidade. Se mudou, enfileira vizinhos fora da nova comunidade que não estão na fila.
3. **Refinamento**: dentro de cada comunidade, partição refinada começa singleton; percorre nós
   em ordem de índice e funde cada nó ainda singleton à sub-comunidade vizinha (dentro da mesma
   comunidade) de maior ganho positivo, empate → menor id. (Versão gulosa, sem a escolha
   aleatória do artigo: preserva a garantia de sub-comunidades conexas.)
4. **Agregação** pela partição refinada, partição inicial do grafo agregado = a do passo 2.
   Repete 2-4 até nenhuma mudança no movimento local.
5. Ao final, separar cada comunidade em componentes conexos (garantia de conexidade explícita).
6. `iterations`: repete 1-5 partindo do resultado anterior (o CRG usa `n_iterations=2`).
7. Renumerar comunidades pela ordem do menor índice de nó.

- [ ] **Step 1: testes que falham**
  - `two_cliques_with_bridge_split_in_two`: dois K5 ligados por uma aresta → 2 comunidades de 5.
  - `disconnected_components_never_share_a_community`.
  - `deterministic`: duas chamadas → mesmo vetor.
  - `higher_resolution_gives_more_communities`: anel de 6 K4 → `γ=0.5` dá ≤ comunidades que `γ=2.0`.
  - `empty_and_edgeless`: `n=0` → `[]`; 3 nós sem arestas → `[0,1,2]`.
  - `modularity_not_worse_than_singletons` em grafo aleatório fixo (gerado por LCG no teste).
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar.
- [ ] **Step 4:** `cargo test -p codegraph-graph leiden` verde.
- [ ] **Step 5:** commit `feat(graph): deterministic Leiden community detection`.

### Task 6: detecção de comunidades (pesos, testes, fallback, split, coesão)

**Files:**
- Modify: `crates/codegraph-graph/src/analysis/communities/mod.rs`
- Test: `crates/codegraph-graph/tests/analysis_communities_test.rs`

**Interfaces:**
- Consumes: `AnalysisGraph`, `leiden`.
- Produces:
  ```rust
  pub struct Community { pub name: String, pub level: i64, pub cohesion: f64, pub size: usize,
      pub dominant_language: String, pub description: String, pub members: Vec<String>,
      pub detector: &'static str /* "leiden" | "file" */ }
  pub fn detect_communities(g: &AnalysisGraph, min_size: usize) -> Vec<Community>; // min_size 2
  ```
  (O nome é preenchido pela Task 7; aqui `name = String::new()`.)

Regras (communities.py:390-604, 836-878):
- Vértices: nós com `kind != "File"`, em ordem de `id`. Arestas: todas as kinds, ambos os lados
  vértices, sem laço; par normalizado `(min,max)`; **primeira ocorrência** fixa o peso
  `CALLS 1.0, IMPORTS_FROM 0.5, INHERITS 0.8, IMPLEMENTS 0.7, CONTAINS 0.3, TESTED_BY 0.4,
  DEPENDS_ON 0.6`, outro 0.5.
- Sem aresta utilizável → fallback por diretório (abaixo), detector `file`.
- `resolution = max(0.05, 1 / log10(max(n, 10)))`; `leiden(g, resolution, 2)`.
- `_reassign_test_nodes` (C:271): teste = `kind == "Test" || is_test`. Índice nome→qn (`None` se
  ambíguo). Para cada aresta `TESTED_BY` (direção ignorada) com exatamente um lado teste e o
  outro resolvido (qn exato, depois nome único), adiciona o outro aos sujeitos do teste. Voto =
  nº de sujeitos distintos por comunidade; empate mantém a atual se empatada, senão menor id.
  Reconstruir preservando ordem; comunidades podem esvaziar.
- Descartar comunidades com `len < min_size`. `dominant_language` = linguagem não vazia mais
  comum (empate: primeira vista), senão `""`. `description = "Community of {n} nodes"`.
- Fallback por diretório (C:513): partes de diretório do caminho (`\`→`/`, sem o arquivo, sem
  vazios); prefixo comum mais longo; chave na profundidade `d` = `remainder[..d].join("/")`, ou o
  stem do arquivo se `remainder` vazio; `d = 1..=max_depth` até ≥10 grupos com `size >= min_size`
  (senão fica a última; `max_depth = 0` → 1). `description = "Directory-based community: {dir}"`.
- `_split_oversized` (C:604): `threshold = max(total·0.25 as int, 10)`; comunidade maior que o
  limiar e com ≥10 nós e arestas internas → Leiden no subgrafo com `resolution=0.5` (pares com
  peso **máximo**, pares ordenados), reassign de testes com todos os nós e arestas, sub-
  comunidades não vazias; ≤1 sub → mantém. Cada sub: `level = pai+1`, `dominant_language` do
  pai, `description = "Split from {nome_pai}"`, coesão sem arredondar; nome fica para a Task 7
  (regra do fallback `"{pai}-{id}"` lá). Uma passada só, não recursivo. Sem igraph no CRG o split
  não roda; aqui roda sempre que o detector for `leiden`.
- Coesão (C:197): por aresta (todas, com repetição): mesmos lados na mesma comunidade →
  `internal += 1`; senão `external += 1` para cada lado que tem comunidade.
  `cohesion = internal / (internal + external)` ou 0; arredondar a 4 (meio-par), exceto split.

- [ ] **Step 1: testes que falham**: `two_modules_two_communities`, `file_nodes_excluded`,
  `first_edge_kind_sets_weight`, `test_node_follows_its_subjects`, `singletons_dropped`,
  `edgeless_graph_uses_directory_fallback` (dois diretórios, 3 nós cada → 2 comunidades
  `detector == "file"`), `text_targets_ignored` (Review Focus 5), `cohesion_counts_external_both_sides`,
  `oversized_community_is_split` (40 nós: 4 grupos densos fracamente ligados forçados numa
  comunidade via resolução baixa — testar `split_oversized` diretamente), `empty_graph_no_communities`.
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar.
- [ ] **Step 4:** verde.
- [ ] **Step 5:** commit `feat(graph): detect communities with test reassignment, fallback and split`.

### Task 7: nomes de comunidade

**Files:**
- Create: `crates/codegraph-graph/src/analysis/communities/naming.rs`
- Test: unitários em `naming.rs`

**Interfaces:**
- Consumes: `Community`, `AnalysisGraph`.
- Produces: `pub fn name_communities(g: &AnalysisGraph, comms: &mut [Community]);` (gera e deduplica).

Regras (communities.py:66-196, 770-835), literais:
- `_COMMON_WORDS`: `get set self init new create update delete add remove make build from to for
  with the and test main run do is has on of in at by my this that all none should when then
  given return returns raise raises expect expected assert tests be it if not`.
- `split_name`: inserir `_` entre `[a-z][A-Z]`, separar em `[_\-.\s]+`, sem vazios.
- `to_slug`: `[^A-Za-z0-9]+` → espaço, `split_name`, lowercase, juntar com `-`; acima de 30
  chars, cortar no último `-` de `slug[..31]` se índice > 0, senão em 30.
- `file_prefix`: por membro, `segmentos[-2]` se ≥2 segmentos, senão stem; o mais comum (empate:
  primeiro visto) → `to_slug`.
- `keywords`: só kinds Function, Class, Test, Type; palavras de `split_name` lowercase, len > 1,
  fora de `_COMMON_WORDS`; top 5 por contagem (empate: primeira vista).
- Nome: membros de nomeação = não-teste (ou todos se todos são teste); vazio → `"empty"`; Class
  mais comum com contagem > 0.4·len → `prefix-slug(class)` ou `slug(class)`; senão
  `prefix-kw`, `prefix`, `kw`, `"cluster"`. Split com nome `""`/`"empty"`/`"cluster"` →
  `"{pai}-{n}"` (n = 1, 2, … na ordem dos splits).
- Dedupe: agrupar por nome; ordenar por `(-size, posição)`; o primeiro mantém; os demais tentam
  `base-slug(kw)` pelos keywords (pula sufixo vazio ou já palavra da base, pula nome tomado);
  senão `base-2`, `base-3`, …

- [ ] **Step 1: testes que falham**: `split_name("parseHTTPRequest") == ["parse", "HTTPRequest"]`,
  `to_slug` com 40 chars, `dominant_class_names_community`, `keyword_with_prefix`,
  `all_tests_still_named`, `duplicates_get_keyword_suffix`, `duplicates_fall_back_to_number`.
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar.
- [ ] **Step 4:** verde.
- [ ] **Step 5:** commit `feat(graph): name communities like code-review-graph`.

### Task 8: gravar comunidades e ferramentas de comunidade

**Files:**
- Modify: `analysis/communities/mod.rs` (`store_communities`), `analysis/mod.rs`
  (`run_postprocess` chama detecção+nomes+store), `graph_query/analysis_queries.rs`, schemas,
  executor, `official_server.rs`
- Test: `analysis_communities_test.rs`, `reconciliation_test.rs`, executor

**Interfaces:**
- Consumes: `detect_communities`, `name_communities`.
- Produces:
  ```rust
  pub fn store_communities(tx: &rusqlite::Transaction, comms: &[Community]) -> anyhow::Result<usize>;
  // GraphQuery
  pub async fn list_communities(&self, sort_by: &str, min_size: usize) -> Result<Vec<CommunityRow>>;
  pub async fn get_community(&self, id: Option<i64>, name: Option<&str>) -> Result<Option<CommunityRow>>;
  pub async fn architecture_overview(&self) -> Result<ArchitectureOverview>;
  // CommunityRow: id, name, level, cohesion, size, dominant_language, description, members: Vec<String>
  ```

- `store_communities` (C:1055): `DELETE FROM communities`, `UPDATE nodes SET community_id = NULL`,
  `INSERT INTO communities (name, level, cohesion, size, dominant_language, description)` (sem
  `parent_id`, como o CRG), `UPDATE nodes SET community_id = ? WHERE qualified_name IN (...)` em
  lotes de 450; `metadata.community_detector` = detectores distintos ordenados juntos por `+`.
- `list_communities` (C:1134): `sort_by ∈ {size, cohesion, name}` (outro → size), size/cohesion
  desc, name asc, desempate `id`; filtro `size >= min_size`; membros por `community_id` em ordem
  de `id`. Tool: `sort_by="size"`, `min_size=0`, `detail_level="standard"`, `max_results=50`
  (teto 200), `max_members=10` (teto 25; cortado → `members_total`, `members_truncated: true`);
  `minimal` → `{name, size, cohesion}`. Resumo `"Found {total} communities[, showing..]"`.
- `get_community_tool`: `community_id` primeiro, senão primeira (ordem size desc) cujo nome
  contém o texto sem caixa; `include_members=false` (true → `member_details` com
  `NodeInfo` dos membros), `max_members=25`. Resumo `"Community '{name}': {size} nodes, cohesion
  {c:.4}"`; não achou → `not_found`, `"No community found matching the given criteria."`.
- `get_architecture_overview` (C:1203): arestas exceto `TESTED_BY` entre comunidades diferentes →
  `{source_community, target_community, edge_kind, source, target}`; pares normalizados
  contados; aviso `"High coupling ({count} edges) between '{n1}' and '{n2}'"` para `count > 10`,
  pulando nomes que casam `(?i)(^test[-/]|[-/]test([:/]|$)|it:should|describe:|spec[-/]|[-/]spec$)`.
  Tool: `detail_level="minimal"`, `max_results=100` (teto 200), `max_members=10`; `minimal`
  agrega por par: `{source_community, target_community, edge_count, top_kinds (3)}`, ordem por
  contagem desc, desempate pelo par.

- [ ] **Step 1: testes que falham**: `store_sets_community_id_and_detector`,
  `list_communities_sorting_and_member_cap`, `get_community_by_name_substring`,
  `overview_warns_on_high_coupling_skips_test_communities`, e em `reconciliation_test.rs`
  `community_ids_stable_across_finalize` (Review Focus 3: mesmo `community_id` por
  `qualified_name` depois de `update` sem mudança).
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar; plugar no `run_postprocess` (`Full`).
- [ ] **Step 4:** `cargo test -p codegraph-graph && cargo test -p codegraph-mcp-tools && cargo test -p codegraph-mcp-server --lib && cargo test -p codegraph-mcp --test reconciliation_test` verdes.
- [ ] **Step 5:** commit `feat(mcp): store communities and add community tools`.

### Task 9: `get_hub_nodes` no formato CRG e `get_bridge_nodes`

**Files:**
- Create: `crates/codegraph-graph/src/analysis/insights.rs`
- Modify: `graph_query/mod.rs:323` (hub atual), `analysis_queries.rs`, schemas, executor, server
- Test: `crates/codegraph-graph/tests/analysis_insights_test.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct HubRow { name, qualified_name, kind, file, in_degree: usize, out_degree: usize,
      total_degree: usize, community_id: Option<i64> }
  pub struct BridgeRow { name, qualified_name, kind, file, betweenness: f64, community_id: Option<i64> }
  pub fn hub_nodes(g: &AnalysisGraph) -> Vec<HubRow>;
  pub fn bridge_nodes(g: &AnalysisGraph) -> Vec<BridgeRow>;
  pub fn betweenness(n: usize, adj: &[Vec<usize>], sources: Option<&[usize]>) -> Vec<f64>;
  ```

- Hubs (analysis.py:27-68): in/out por aresta (todas as kinds, com repetição) cujo lado é nó;
  candidatos não-File com grau > 0; ordenar `total_degree` desc, desempate `id`.
  **Mudança de contrato**: a tool `get_hub_nodes` passa a aceitar `top_n=10` (teto 100) e
  `detail_level` (`minimal` → `{name, kind, total_degree}`); `min_degree` continua aceito como
  filtro opcional para não quebrar quem usa. Atualizar `src/prompts/*.md` e
  `rules-for-claude-code/codegraph_rule.md` se citarem o formato antigo.
- Bridges (analysis.py:71-127): DiGraph de todas as arestas, pares paralelos colapsados, pontas
  que não são nó **também** entram como vértices (texto, como o networkx faz); Brandes não
  ponderado, dirigido, normalizado por `1/((n-1)(n-2))`; > 5000 vértices → amostra (Decisão 4).
  Manter score > 0 e nó não-File; `betweenness = round_half_even(s, 6)`; desc, desempate `id`.
  Tool `get_bridge_nodes`: `top_n=10` (teto 100), `detail_level` (`minimal` → `{name, kind,
  betweenness}`); resumo `"{total} bridge node(s) ranked by betweenness[, showing..]"`.

- [ ] **Step 1: testes que falham**: `hub_degrees_count_parallel_edges`,
  `betweenness_path_graph` (a→b→c: b = 0.5 normalizado), `bridge_between_two_clusters_ranks_first`,
  `bridges_on_crg_written_db` (fixture sem `cg_nodes`, Review Focus 1), `sampled_betweenness_is_deterministic`
  (grafo de 6000 nós gerado; duas execuções iguais).
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar; adaptar o executor/tool existentes de hub.
- [ ] **Step 4:** verdes (graph, mcp-tools, server lib).
- [ ] **Step 5:** commit `feat(mcp): CRG-shaped get_hub_nodes and new get_bridge_nodes`.

### Task 10: `get_knowledge_gaps` e `get_surprising_connections`

**Files:**
- Modify: `analysis/insights.rs`, `analysis_queries.rs`, schemas, executor, server
- Test: `analysis_insights_test.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct KnowledgeGaps { isolated_nodes, thin_communities, untested_hotspots,
      single_file_communities, import_cycles }   // Vec de structs com os campos do CRG
  pub fn knowledge_gaps(g: &AnalysisGraph, communities: &[CommunityRow]) -> KnowledgeGaps;
  pub struct SurprisingRow { source, source_qualified, target, target_qualified, edge_kind,
      surprise_score: f64, reasons: Vec<&'static str>, source_community, target_community }
  pub fn surprising_connections(g: &AnalysisGraph) -> Vec<SurprisingRow>;
  ```

- Gaps (analysis.py:148-346): grau não dirigido sobre todas as arestas; `tested` = origens de
  `TESTED_BY`. `isolated_nodes`: não-File com grau ≤ 1 (ordem de `id`, máx. 50).
  `thin_communities`: membros não-File < 3 → `{community_id, name, size}`. `untested_hotspots`:
  grau ≥ 5, não testado, não teste; grau desc, máx. 20. `single_file_communities`: membros num só
  arquivo e `size >= 3` → `{community_id, name, size, file}`. `import_cycles`: grafo de
  `IMPORTS_FROM` entre nós `File`, sem auto-import; `petgraph::algo::tarjan_scc`, componentes com
  > 1 arquivo; laço mostrado = BFS do menor arquivo em ordem alfabética, vizinhos ordenados;
  campos `name` (`"a -> b -> a"`), `size`, `load_time`, `files[..10]`, `loop_length`,
  `cycle[{file, line, imports}][..10]` (`line` = menor linha do import); ordem
  `(!load_time, -size, name)`. Import só de tipo/`TYPE_CHECKING`/pacote pai: o Rust já descarta
  imports de `TYPE_CHECKING` no parser; os demais casos (`import type`, pacote pai): ler
  analysis.py:148-244 e portar o que der com os dados de `edges.extra`; o que não der, registrar
  no PLANO. `load_time = true` sempre que não houver dado de import adiado.
  Tool: `max_per_category=15` (teto 50), `detail_level`; resposta `{summary: {cat: count}, gaps,
  total_gaps, truncated}`.
- Surprising (analysis.py:349-450): `thr = max(mediana_superior(graus > 0) · 3, 10)`; por aresta
  com dois lados não-File: `+0.3 cross-community` (ambos com id e diferentes), `+0.2
  cross-language` (extensões não vazias e diferentes), `+0.2 peripheral-to-hub`, `+0.15
  cross-test-boundary` (`is_test` difere e `CALLS`), `+0.15 unusual-edge-kind` (`CALLS` e origem
  `Type`); mantém score > 0; `round_half_even(s, 2)`; desc, desempate pela ordem da aresta.
  Tool: `top_n=15` (teto 100), `detail_level` (`minimal` → `{source, target, edge_kind,
  surprise_score}`).

- [ ] **Step 1: testes que falham**: um por categoria de gap, `import_cycle_shortest_loop`,
  `surprise_reasons_and_score`, `gaps_on_empty_graph_are_empty` (Review Focus 2).
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar.
- [ ] **Step 4:** verdes.
- [ ] **Step 5:** commit `feat(mcp): add get_knowledge_gaps and get_surprising_connections`.

### Task 11: `get_suggested_questions`

**Files:**
- Modify: `analysis/insights.rs`, `analysis_queries.rs`, schemas, executor, server
- Test: `analysis_insights_test.rs`

**Interfaces:**
- Consumes: `bridge_nodes`, `hub_nodes`, `surprising_connections`, `knowledge_gaps`.
- Produces: `pub fn suggested_questions(..) -> Vec<Question{category, question, target, priority}>`.

Ordem (analysis.py:453-546): 3 bridges (`bridge_node`, high), 3 hubs sem `TESTED_BY` como origem
(`hub_risk`, high), 3 surprising com `cross-community` (`surprising_connection`, medium, alvo
`source_qualified`), `thin_communities[..2]` (`thin_community`, low, alvo `"community:{id}"`),
`untested_hotspots[..2]` (`untested_hotspot`, medium). Textos das perguntas: copiar as f-strings
de analysis.py:473-541 literalmente. Tool sem parâmetros; resposta `{questions, count,
by_priority: {high, medium, low}}`.

- [ ] **Step 1:** teste `questions_in_crg_order_with_literal_text` com um fixture que gera ao
  menos uma de cada categoria; comparar strings exatas.
- [ ] **Step 2:** rodar → falha. **Step 3:** implementar. **Step 4:** verde.
- [ ] **Step 5:** commit `feat(mcp): add get_suggested_questions`.

### Task 12: risk score no review

**Files:**
- Create: `crates/codegraph-graph/src/analysis/risk.rs`
- Modify: `analysis_queries.rs`, `crates/codegraph-mcp-tools/src/review.rs:24,90-104,116-130`,
  schemas de `detect_changes`/`get_review_context`/`get_minimal_context`
- Test: unitários em `risk.rs`, `review.rs` tests

**Interfaces:**
- Produces:
  ```rust
  pub struct RiskSignals { pub flows: f64, pub cross_community: f64, pub tests: f64,
      pub security: f64, pub callers: f64, pub churn: f64 }
  pub fn risk_signals(g: &AnalysisGraph, ix: usize, flow_criticalities: &[f64],
      churn_commits: Option<u32>) -> RiskSignals;
  pub fn risk_score(s: &RiskSignals) -> f64; // round_half_even(clamp(Σ, 0, 1), 4)
  // GraphQuery
  pub async fn risk_scores(&self, ids: &[String], churn: &HashMap<String, u32>)
      -> Result<HashMap<String, (f64, RiskSignals)>>;
  ```

Sinais (changes.py:1275-1454):
```
flows           = if !crits.is_empty() { min(Σcrits, 0.25) } else { min(flow_count·0.05, 0.25) }
cross_community = min(#chamadores (CALLS de entrada) com community_id ≠ None e ≠ do nó · 0.05, 0.15)
                  (0 se o nó não tem comunidade)
tests           = 0.30 − min(test_count / 5, 1) · 0.25
                  test_count = testes de TESTED_BY do nó (Class: também dos membros CONTAINS;
                  File: dos Class/Function), mais os dos callees a 1 salto de CALLS, fronteira ≤ 50
                  (graph.py:2316 get_transitive_tests; ler para os detalhes de "unresolved")
security        = 0.20 se name/qualified (lowercase) contém SECURITY_KEYWORD, senão 0
callers         = min(#arestas CALLS de entrada / 20, 0.10)
churn           = min(commits / 10, 1) · 0.15   (só com churn; commits em 90 dias do arquivo)
```
Review: `ReviewNode.risk: usize` vira `risk_score: f64` (e `risk_signals` no `standard`);
ordenação por `risk_score` desc, depois nome, depois id. Topo do resultado ganha `risk_score`
(máximo) e `risk` = `"high"` se > 0.7, `"medium"` se > 0.4, senão `"low"`. `detect_changes`
ganha `risk_score` por nó e `review_priorities` (top 10). Churn: `git log --since=90.days
--format= --name-only <base>..` uma vez por chamada, contagem por arquivo, timeout 5 s, máx.
2000 commits; erro de git → sem churn e `churn_status: "unavailable"`. Parâmetro
`include_churn` (padrões na Decisão 8).

- [ ] **Step 1: testes que falham**: `untested_security_function_scores_high`
  (`0.30 + 0.20 = 0.50`, sem flows), `flow_signal_caps_at_quarter`, `cross_community_callers`,
  `well_tested_low_risk` (5 testes → `tests = 0.05`), `churn_saturates`, e no review
  `nodes_ordered_by_risk_score` + `risk_level_thresholds`.
- [ ] **Step 2:** rodar → falha.
- [ ] **Step 3:** implementar; atualizar as descrições das 3 tools (hoje dizem "dependents plus
  one when untested").
- [ ] **Step 4:** `cargo test -p codegraph-graph && cargo test -p codegraph-mcp-tools && cargo test -p codegraph-mcp-server --lib` verdes.
- [ ] **Step 5:** commit `feat(review): CRG risk score with flows, communities, tests and churn`.

### Task 13: paridade com o CRG, docs e fechamento

**Files:**
- Modify: `scripts/parity/parity.py`, `docs/parity/baseline.md`, `docs/architecture/storage-sqlite.md`,
  `CLAUDE.md` (lista de ferramentas), `src/prompts/cli_instructions.md`, `PLANO.md`

- [ ] **Step 1:** `parity.py` ganha seções:
  - flows: Jaccard dos `qualified_name` de entry points; para flows com o mesmo entry, Jaccard
    dos membros e |Δ criticality|; resumo por linguagem.
  - comunidades: ARI (Adjusted Rand Index) entre `community_id` dos nós comuns, e nº de
    comunidades de cada lado; `community_detector` de cada banco.
- [ ] **Step 2:** rodar `python3 scripts/parity/parity.py corpus` e a sonda Python; gravar os
  números em `docs/parity/baseline.md`. Critério: entry points e membros de flow ≥ 0.95 onde
  CALLS já é 1.00; ARI só registrado (Decisão 3).
- [ ] **Step 3:** rodar `target/debug/codegraph build .` neste repo, medir o tempo do
  pós-processo e o tamanho do JSON de `get_architecture_overview` (minimal); registrar no PLANO.
- [ ] **Step 4:** docs: tabelas `flows`/`communities` passam a ser escritas (storage-sqlite.md),
  ferramentas novas no CLAUDE.md e no guia de CLI, `--skip-flows`/`--skip-postprocess`.
- [ ] **Step 5:** PLANO.md: Fase 4 ✅ com o que ficou para depois (incremental de flows, igualdade
  com igraph, gaps de import não portados).
- [ ] **Step 6:** `cargo test --workspace --no-fail-fast` (pulando os testes de watcher do WSL
  listados no PLANO) e `cargo check --workspace --all-targets --all-features` limpos.
- [ ] **Step 7:** commit `docs: close Fase 4 (flows, communities, insights, risk)`; depois
  `cargo clean`.

---

## Riscos

- Leiden à mão: maior risco de bug sutil. Os testes da Task 5 checam propriedades, não uma
  partição fixa; se a Task 13 mostrar ARI muito baixo no corpus, comparar a modularidade dos dois
  lados antes de suspeitar do algoritmo.
- Betweenness exato é O(V·E): em ~5000 nós com ~35k arestas (este repo) cabe em segundos; medir
  na Task 9.
- Mudança de contrato de `get_hub_nodes` (Task 9) pode quebrar skills/hooks que leem o formato
  antigo; procurar usos com `rg get_hub_nodes` antes de mudar.
- Pós-processo dentro do lock de escrita aumenta o tempo do `update` disparado pelo hook de git;
  `--skip-flows` existe para isso.
