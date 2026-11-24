# Higiene: sufixo `@L1` nos nós e arquivo LICENSE — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Nós gravados com o `qualified_name` limpo depois que o dono antigo do nome some, e o repositório com arquivos de licença coerentes.

**Architecture:** `write_node` dá `base@L<line>` quando outro UUID ainda ocupa `base` (típico logo após mudar o hash dos IDs ou `GRAPH_SHAPE_VERSION`: as linhas antigas só saem na reconciliação, depois da escrita). Como o nome com sufixo é mantido para sempre (`write.rs:130`), o sufixo nunca some. Correção: `cg_nodes` guarda o nome-base; o `finalize` renomeia (nós, `cg_nodes` e arestas) toda linha com sufixo cujo base ficou livre.

**Tech Stack:** Rust, rusqlite, SQLite.

**Spec:** `PLANO.md`, seção "Fase 4 → Fica para depois" (item `@L1`) e "Decisões em aberto" item 4 (licença).

## Global Constraints

- `schema_v15.sql` não se edita; só `schema_cg.sql` (tabelas `cg_*`).
- Nomes `...@L<n>` legítimos (nós `Test` de JS: `a.test.ts::it:x@L2`) não podem ser renomeados: só vale a coluna de base gravada, nunca regex no nome.
- Novos arquivos `.rs` começam com duas linhas `// ABOUTME:`.
- `cargo` sempre com `--workspace` ou `-p`. Commits terminam com `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Rebuild forçado de grafos antigos via `GRAPH_SHAPE_VERSION` (`crates/codegraph-mcp/src/indexer.rs:45`, hoje `crg-shape-9`).

## Review Focus

- Dois nós com o mesmo base legítimo no mesmo arquivo (mesmo nome, linhas diferentes): ambos precisam continuar distintos (`base`, `base@L<line>`), sem colisão ao normalizar.
- Nó `Test` de JS cujo nome termina em `@L2`: não renomeado.
- Banco antigo sem a coluna nova: abre sem erro e é reconstruído.
- Normalização repetida (idempotente) e sem efeito num grafo já limpo.
- Arestas apontando para o nome antigo seguem válidas depois do rename.

---

### Task 1: Reproduzir o `@L1` com teste

**Files:**
- Test: `crates/codegraph-graph/tests/sqlite_write_test.rs`

**Interfaces:**
- Consumes: `SqliteGraphStore::write_nodes`/`write_edges`, `finalize`, `reconcile` (ver `tests/sqlite_reconcile_test.rs` para os helpers `node`, `edge`, `file_meta`).
- Produces: teste `stale_uuid_does_not_leave_suffixed_name`.

- [ ] **Step 1: Escrever o teste que falha.** Gravar o nó `A` (uuid 1, `f.py::g`, linha 1); gravar o nó `B` (uuid 2, mesmo arquivo e nome, linha 1) enquanto `A` ainda existe; reconciliar removendo `A` (arquivo atualizado só com `B`); `finalize`. Asserts: `SELECT qualified_name FROM nodes WHERE name='g'` devolve exatamente `<root>/f.py::g` (sem `@L`), e `cg_nodes` idem para o uuid 2.
- [ ] **Step 2: Rodar:** `cargo test -p codegraph-graph --test sqlite_write_test stale_uuid_does_not_leave_suffixed_name`. Esperado: FAIL com `...f.py::g@L1`. Se passar, o diagnóstico está errado: parar e reinvestigar com o banco do repo (`sqlite3 .code-review-graph/graph.db`; 174 `File` com `@L1`).
- [ ] **Step 3: Commit** do teste vermelho só se a suíte do crate tolerar; senão deixar para o commit da Task 2.

### Task 2: Guardar o base e normalizar no `finalize`

**Files:**
- Modify: `crates/codegraph-graph/src/sqlite_store/schema_cg.sql` (coluna `base_qualified TEXT` em `cg_nodes`)
- Modify: `crates/codegraph-graph/src/sqlite_store/schema.rs` (`ensure`: `ALTER TABLE cg_nodes ADD COLUMN base_qualified TEXT` quando `PRAGMA table_info(cg_nodes)` não a lista)
- Modify: `crates/codegraph-graph/src/sqlite_store/write.rs` (`write_node`: gravar `base` na coluna, tanto no INSERT quanto no `ON CONFLICT(uuid)`)
- Modify: `crates/codegraph-graph/src/sqlite_store/reconcile.rs` (`finalize_with`: chamar a normalização depois de `resolve_pending` e antes do `rebuild` do FTS)
- Modify: `crates/codegraph-mcp/src/indexer.rs:45` (`GRAPH_SHAPE_VERSION` → `crg-shape-10`, para reescrever os nós de bancos antigos e preencher a coluna)
- Test: `crates/codegraph-graph/tests/sqlite_write_test.rs`

**Interfaces:**
- Produces: `pub(crate) fn normalize_suffixed_names(tx: &Transaction) -> Result<usize>` em `write.rs` (nº de nós renomeados).

- [ ] **Step 1: Testes que falham** (todos em `sqlite_write_test.rs`): o da Task 1; `test_node_named_with_l_suffix_is_kept` (nó `Test` com nome `it:x@L2` continua com esse nome após `finalize`); `colliding_bases_stay_distinct` (dois nós, uuids 3 e 4, mesmo base, linhas 1 e 9, ambos vivos: ficam `base` e `base@L9`); `normalize_is_idempotent` (duas chamadas, a segunda devolve 0); `edges_follow_rename` (aresta `A→B` com `B` renomeado: `edges.target_qualified` igual ao novo nome).
- [ ] **Step 2: Rodar e ver falhar.**
- [ ] **Step 3: Implementar `normalize_suffixed_names`.** Para cada linha de `cg_nodes` com `base_qualified IS NOT NULL AND qualified_name != base_qualified`, em ordem de `qualified_name`, e só se nenhuma outra linha de `cg_nodes` tem `qualified_name = base_qualified`: `UPDATE` do nome em `nodes` (pela chave antiga), em `cg_nodes` e em `edges.source_qualified`/`target_qualified`; recalcular `symbol` em `nodes` como em `write_node` (parte depois de `::`). Reverificar a ocupação a cada linha (colisão entre dois sufixados do mesmo base: o primeiro ganha). Linhas com `base_qualified` nulo são ignoradas.
- [ ] **Step 4: Rodar** `cargo test -p codegraph-graph` e `cargo test -p codegraph-mcp --lib`. Esperado: tudo verde.
- [ ] **Step 5: Verificação real.** `cargo build -p codegraph-mcp-server --bin codegraph`; rodar `codegraph build .` neste repo e conferir `sqlite3 .code-review-graph/graph.db "select count(*) from nodes where kind='File' and qualified_name like '%@L%'"` = 0 e IMPORTS_FROM `File→File` bem acima de 141.
- [ ] **Step 6: Commit** (`fix(store): drop @L suffix once the stale node owning the name is gone`).

### Task 3: Arquivos de LICENSE

**Decisão necessária antes de começar (perguntar ao usuário):** o `Cargo.toml` do workspace diz `MIT OR Apache-2.0`, três crates (`codegraph-mcp-daemon`, `codegraph-mcp-server`, `codegraph-mcp-tools`) dizem só `Apache-2.0`, e o `README.md:1017` diz `MIT`. Escolher: (a) dual MIT OR Apache-2.0 em tudo; (b) só MIT; (c) só Apache-2.0. Também o titular do copyright: manter o do upstream (`Jakedismo/codegraph-rust`) e acrescentar o do fork, ou outro.

**Files:**
- Create: `LICENSE-MIT` e/ou `LICENSE-APACHE` (texto padrão da licença escolhida, copyright conforme a decisão)
- Modify: `Cargo.toml:24` e os `license` dos 3 crates divergentes (usar `license.workspace = true`)
- Modify: `README.md:1017-1019` (seção License)

- [ ] **Step 1: Conferir o upstream** (`git remote` `upstream`; `gh api repos/Jakedismo/codegraph-rust/license`) para não contradizer a licença original.
- [ ] **Step 2: Criar os arquivos e alinhar os manifests e o README** à escolha.
- [ ] **Step 3: Verificar:** `cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys;print({p['name']:p['license'] for p in json.load(sys.stdin)['packages']})"` mostra a mesma licença em todos os 8 crates; `cargo check --workspace`.
- [ ] **Step 4: Commit** (`docs: add license files and align crate manifests`).

### Task 4: Fechar no PLANO.md

**Files:**
- Modify: `PLANO.md` (remover o `@L1` de "Fica para depois" e de "Gaps de import"; marcar a decisão 4 como resolvida; atualizar `GRAPH_SHAPE_VERSION` na Fase 2.5 item 1 para `crg-shape-10`)

- [ ] **Step 1: Editar e commitar** (`docs: close @L1 suffix and license items`). Depois `cargo clean`, conforme a memória do projeto (target/ chegou a 39 GB).

---

## Self-review

- Cobertura: `@L1` (Tasks 1-2), licença (3), plano (4). O sintoma "IMPORTS_FROM entre `File`s" é medido no Step 5 da Task 2.
- Risco de hipótese: a causa (UUID antigo ainda dono do nome na hora da escrita) foi deduzida do código, não reproduzida; a Task 1 a confirma ou derruba antes de qualquer correção.
