# Cache do AnalysisGraph e do churn Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chamadas repetidas de análise e de review não recarregam o grafo nem rodam `git log` de novo, sem mudar resultados.

**Architecture:** `GraphQuery` ganha um segundo cache (como o do `Snapshot`) para `Arc<AnalysisGraph>`; `git_changes.rs` ganha um cache de processo para o churn por (raiz, HEAD) e um conjunto de raízes com timeout.

**Tech Stack:** Rust, rusqlite, tokio, git CLI.

**Spec:** `docs/superpowers/specs/2026-11-24-cache-analise-churn-design.md`

## Global Constraints

- Nenhum resultado de ferramenta muda (mesmos JSONs); só tempo e carga.
- `cargo` sempre com `-p`/`--workspace`. Novos `.rs` começam com duas linhas `// ABOUTME:`. Commits terminam com `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Cache de churn: no máximo 32 entradas; só timeout é persistente; HEAD via `git rev-parse HEAD` com orçamento de 5 s.

## Review Focus

- Escrita por outra conexão/processo entre duas chamadas: o grafo em cache não pode ser servido (marcador mudou).
- `risk_scores` carrega o grafo dentro de uma transação de leitura: o grafo do cache tem de ser do mesmo estado que o `Snapshot` dessa transação.
- Repositório sem commits: churn `unavailable` sem ficar preso depois do primeiro commit.
- Raiz acessada por symlink: contagens de churn encontradas por nó com caminho canônico e com caminho do symlink.
- Dois repositórios no mesmo processo não compartilham entradas de churn.

---

### Task 1: Cache do `AnalysisGraph` em `GraphQuery`

**Files:**
- Modify: `crates/codegraph-graph/src/graph_query/mod.rs` (campo de cache + helper), `crates/codegraph-graph/src/graph_query/analysis_queries.rs` (5 chamadas de `AnalysisGraph::load` nas ferramentas e a de `risk_scores`)
- Test: `crates/codegraph-graph/tests/graph_query_test.rs`

**Interfaces:**
- Produces: `fn analysis_graph_in(conn: &rusqlite::Connection, cache: &Mutex<AnalysisCached>) -> Result<Arc<AnalysisGraph>>` (mesma forma de `snapshot_in`, chave = `change_marker(conn)`); `#[doc(hidden)] pub fn analysis_graph_loads(&self) -> usize` em `GraphQuery` (contador de cargas reais, para teste).

- [ ] **Step 1: Testes que falham** em `graph_query_test.rs` (reaproveitar o fixture do arquivo): `analysis_graph_is_loaded_once_across_tools` (chamar `hub_nodes`, `bridge_nodes`, `knowledge_gaps`, `suggested_questions`, `surprising_connections` em sequência sem escrever: `analysis_graph_loads() == 1`); `analysis_graph_reloads_after_a_write` (depois de `upsert_nodes_batch` de um nó novo por `query.store()`, a próxima ferramenta de análise eleva o contador para 2 e `hub_nodes` reflete o nó); `analysis_graph_reloads_after_another_connection_writes` (escrita por uma segunda conexão `rusqlite` no mesmo arquivo: contador 2).
- [ ] **Step 2: Rodar** `cargo test -p codegraph-graph --test graph_query_test analysis_graph` → FAIL (método inexistente).
- [ ] **Step 3: Implementar** `AnalysisCached = Option<((i64, i64), Arc<AnalysisGraph>)>`, o campo `analysis_cache: Arc<Mutex<AnalysisCached>>` com contador `Arc<AtomicUsize>` em `GraphQuery` (inicializado em `new`), `analysis_graph_in` e a troca das 6 chamadas por ele. Em `risk_scores`, chamar o helper dentro da transação, depois de `snapshot_in`, para ambos serem do mesmo estado.
- [ ] **Step 4: Rodar** `cargo test -p codegraph-graph` e `cargo test -p codegraph-mcp-tools --lib` → PASS (nenhum teste existente de análise muda de saída).
- [ ] **Step 5: Commit** (`perf(graph): cache the AnalysisGraph by database change marker`).

### Task 2: Cache do churn por (raiz, HEAD) com timeout persistente

**Files:**
- Modify: `crates/codegraph-mcp-tools/src/git_changes.rs` (`file_churn`, novo `file_churn_with`, `clear_churn_cache`)
- Test: o módulo `tests` do mesmo arquivo (repositórios git temporários)

**Interfaces:**
- Produces: `pub async fn file_churn(root: &Path) -> Option<HashMap<String, u32>>` (assinatura igual, agora com cache); `pub fn clear_churn_cache()`; `async fn file_churn_with(root: &Path, budget: Duration) -> Option<HashMap<String, u32>>` (privada, para teste); contador `#[cfg(test)] fn git_log_runs(root: &Path) -> usize`.

- [ ] **Step 1: Testes que falham:** `churn_is_cached_per_head` (repo temporário com 1 commit: duas chamadas rodam `git log` 1 vez; um novo commit muda o HEAD, a terceira chamada roda de novo e conta o arquivo novo); `two_repositories_do_not_share_entries`; `timeout_is_remembered_and_cleared` (`file_churn_with(root, Duration::ZERO)` devolve `None`; uma segunda chamada com orçamento normal devolve `None` sem rodar `git log`; `clear_churn_cache()` e a terceira devolve `Some`); `repository_without_commits_recovers_after_first_commit` (`None`, depois do commit `Some`, nada preso).
- [ ] **Step 2: Rodar** `cargo test -p codegraph-mcp-tools --lib git_changes` → FAIL.
- [ ] **Step 3: Implementar.** Cache global `static CACHE: Mutex<State>` com `entries: HashMap<(PathBuf, String), HashMap<String, u32>>` (ao passar de 32 entradas, esvaziar), `timed_out: HashSet<PathBuf>`; o fluxo é: raiz em `timed_out` → `None` sem git; `git rev-parse HEAD` sob o orçamento (falha ou saída vazia → `None`, não persistente; timeout → persistente); acerto por (raiz, HEAD) devolve o clone; senão roda o `git log` atual, guarda e devolve; timeout do `git log` → persistente.
- [ ] **Step 4: Rodar** `cargo test -p codegraph-mcp-tools --lib` e `cargo test -p codegraph-mcp-tools` → PASS.
- [ ] **Step 5: Commit** (`perf(review): cache churn per repository HEAD and remember git timeouts`).

### Task 3: Caminhos do churn pela raiz canônica

**Files:**
- Modify: `crates/codegraph-mcp-tools/src/git_changes.rs` (montagem das chaves em `file_churn_with`)
- Test: mesmo módulo `tests`

- [ ] **Step 1: Teste que falha** `churn_keys_cover_symlinked_roots` (Unix): repo temporário com um commit tocando `a.py`; `ln -s` para ele; `file_churn(symlink)` contém as chaves `a.py`, `<symlink>/a.py` e `<canônico>/a.py`, todas com contagem 1.
- [ ] **Step 2: Rodar** → FAIL (falta a chave canônica).
- [ ] **Step 3: Implementar:** além de `root.join(path)`, inserir `root.canonicalize()?.join(path)` quando difere (contar uma vez por chave distinta).
- [ ] **Step 4: Rodar** `cargo test -p codegraph-mcp-tools` → PASS; depois rodar `cargo test -p codegraph-mcp-server` (review end-to-end).
- [ ] **Step 5: Medir.** `codegraph build .` neste repo, depois `time codegraph tool run get_review_context --params '{"base_ref":"HEAD~3"}'` duas vezes seguidas no mesmo processo MCP (ou duas chamadas `get_minimal_context` por `codegraph start stdio`); registrar o antes (commit anterior) e o depois no `PLANO.md` (Fase 4, "Fica para depois": remover o cache de `AnalysisGraph` e o de churn; manter o "unavailable persistente" como resolvido só para timeout e as três mudanças de score como pendentes).
- [ ] **Step 6: Commit** (`fix(review): match churn by canonical root; docs: update PLANO.md`). Depois `cargo clean` (memória do projeto).

---

## Self-review

- Cobertura do spec: cache do grafo (T1), cache + timeout do churn (T2), caminho canônico (T3). Mudanças de score ficam fora, como no design.
- Consistência: `file_churn` mantém a assinatura usada por `review.rs` e `graph_tool_executor.rs`; `analysis_graph_loads` só existe para teste.
