# Plano: code-review-graph em Rust

Objetivo: transformar este fork do `codegraph-rust` numa versão em Rust do
[code-review-graph](../code-review-graph) (CRG): um binário único, sem Python e sem LLM, que
indexa o repositório num grafo local e entrega a agentes de IA (via MCP e CLI) contexto de
revisão determinístico e barato em tokens.

Referência de comportamento: `../code-review-graph` (Python, v2.4.0, ~91k linhas, 31
ferramentas MCP, schema SQLite versão 15).

---

## Fase 0 — Limpeza da camada de IA ✅ (branch `phase0-strip`, não commitada)

Feito:

- Removidos os crates `codegraph-mcp-rig` (agente LLM), `codegraph-vector` (embeddings,
  reranking), `codegraph-zerocopy` e `codegraph-concurrent`. Workspace: 12 → 8 crates.
- Removidas as features `ai-enhanced`, `embeddings-*`, `cloud`, `rig-*` e todo o código
  `#[cfg]` correspondente (indexer, resolução semântica, servidor, testes).
- Servidor MCP reescrito (`official_server.rs`): as 4 ferramentas "agentic" deram lugar a 8
  ferramentas de grafo diretas, com parâmetros tipados. Nova ferramenta `find_nodes_by_name`
  para obter `node_id` sem busca semântica.
- `codegraph agent …` → `codegraph tool run|list|instructions` (`tool_cli.rs`).
- Hooks do Claude/Codex, guias de instrução (`src/prompts/*.md`), `CLAUDE.md`, `AGENTS.md`,
  Makefile e workflow de release atualizados. Scripts Python de avaliação agentic removidos.
- −5,1k linhas nos arquivos alterados, além dos crates removidos.

Pendente para fechar a fase:

- [x] `cargo test -p codegraph-mcp --test e2e_mcp_tools` verde (5/5). Suíte completa
      `cargo test --workspace --all-features --no-fail-fast` verde, pulando os 5 testes de
      watcher do WSL (ver notas de ambiente); `cargo check --workspace --all-targets
      --all-features` limpo.
- [x] Commit em `phase0-strip`.
- [ ] Abrir PR para `main` no fork.
- [ ] Abrir issue no upstream (Jakedismo/codegraph-rust) pedindo um arquivo `LICENSE`
      (o `Cargo.toml` declara "MIT OR Apache-2.0", mas não há arquivo de licença).

Sobras conhecidas (sem efeito, a limpar junto com a Fase 1):

- Flags e config de embedding (`--batch-size`, `[embedding]`, `[llm]`, `[rerank]`,
  `CODEGRAPH_EMBEDDING_*`), barras de progresso de embedding no indexer, comando `estimate`
  com ETA de embeddings, tabelas e colunas de embedding no schema SurrealQL.
- `codegraph-core/src/integration/graph_vector.rs` (`GraphVectorIntegrator`,
  `HasherEmbeddingService`), ainda usado por `incremental/updater.rs`.
- `README.md`, `docs/` e banners dos `install-codegraph*.sh` ainda descrevem o produto antigo.

### Notas de ambiente (máquina local, WSL)

- **OpenSSL**: o `pkg-config` do Linuxbrew faz o build linkar com o OpenSSL 4 do brew, cuja
  pasta de bibliotecas não está no caminho do carregador. O binário falha com
  `libssl.so.4: cannot open shared object file`. Para rodar o binário ou os testes:
  `export LD_LIBRARY_PATH=/home/linuxbrew/.linuxbrew/lib`. Alternativa: compilar com
  `PKG_CONFIG=/usr/bin/pkg-config` (OpenSSL 3 do sistema; exige rebuild). A Fase 1 resolve
  isso de vez ao tirar SurrealDB e `reqwest`/`native-tls` do binário.
- **Watchers no WSL**: 2 testes de `codegraph-core::watch` falham também no código original.
  3 testes de watcher de `codegraph-parser` travam (`test_file_change_detection`,
  `test_file_monitoring_only`, `test_integration_test_suite`). Rodar a suíte com
  `--skip` nesses testes até investigar (provável limitação de inotify em WSL).
- **Clippy estrito** (`-D warnings`) falha em diagnósticos que já existiam em `codegraph-core`.

---

## Fase 1 — Storage: SurrealDB → SQLite + petgraph

Por quê: compatibilidade com o banco do CRG, concorrência (SurrealKV trava o diretório para
um único processo, e o CRG roda watch + hooks + MCP ao mesmo tempo), licença (SurrealDB é
BSL), build menor (sem C++/OpenSSL) e testes de paridade diretos entre os dois bancos.

Tarefas:

1. Criar o crate `codegraph-store` (ou reescrever `codegraph-graph`) com `rusqlite`
   (`bundled`), WAL, e o schema do CRG versão 15 (`graph.py:_SCHEMA_SQL` +
   `migrations.py`): `nodes`, `edges`, `metadata`, `flows`, `flow_memberships`,
   `flow_snapshots`, `communities`, `community_summaries`, `risk_index`, FTS de nós
   (`nodes_fts`). Banco em `.code-review-graph/graph.db`, para ler índices existentes.
2. Mapear tipos: `codegraph_core::CodeNode`/`EdgeType` ↔ `nodes.kind` (File, Class, Function,
   Test, …) e `edges.kind` do CRG (CALLS, IMPORTS_FROM, INHERITS, CONTAINS, TESTED_BY, …).
   Documentar as perdas em cada direção.
3. Trocar o `writer.rs` e o `indexer.rs` do `codegraph-mcp` para gravar no SQLite;
   manter a reconciliação incremental (hash por arquivo, remoção de registros obsoletos).
4. Camada de consulta em memória com `petgraph`, carregada sob demanda e invalidada por
   `metadata.last_updated`: dependências transitivas e reversas, ciclos (Tarjan), hubs,
   coupling, call chain. Reimplementar as 8 ferramentas atuais sobre ela, no lugar das
   funções `fn::*` em SurrealQL.
5. Remover `surrealdb`, `schema/*.surql`, `SurrealDbStorage`, `CODEGRAPH_SURREALDB_*`,
   `db-check` e as sobras de embedding listadas na Fase 0.
6. Migrar o lock de processo único para o modelo do CRG (vários leitores, um escritor; WAL).

Critério de aceite: `codegraph index` + `codegraph tool run …` + `codegraph start stdio`
funcionando sem SurrealDB; o binário não depende de `libssl`; as 8 ferramentas com testes
sobre um repositório fixture.

---

## Fase 2 — Paridade do parser

Risco principal do projeto: o `parser.py` do CRG tem ~25k linhas de regras por linguagem,
mais resolvers específicos (C#, AOP, eventos, HCL, aliases npm, workspaces, jedi), contra
~12k linhas no `codegraph-parser`.

1. Harness de paridade (`scripts/parity/`): indexar o mesmo repositório com o CRG (Python) e
   com o binário Rust; comparar os dois `graph.db` por `(kind, qualified_name, file, linhas)`
   para nós e `(kind, source, target)` para arestas. Relatório por linguagem: só-no-CRG,
   só-no-Rust, divergentes.
2. Corpus de teste: repositórios pequenos e fixos por linguagem (começar por Python, TS/JS,
   Go, Rust, Java; depois C#, C/C++, Ruby, PHP, Kotlin, Swift, Dart), mais os fixtures de
   `../code-review-graph/tests`.
3. Fechar as diferenças linguagem a linguagem, priorizando arestas CALLS/IMPORTS e a
   detecção de testes (`is_test`, TESTED_BY), que alimentam o raio de impacto.
4. Avaliar o tier `balanced` (LSP) como substituto dos resolvers heurísticos; registrar em
   que casos o LSP supera o CRG e em quais fica atrás.

Critério de aceite: ≥95% de concordância de nós e ≥90% de arestas CALLS/IMPORTS nas
linguagens prioritárias, com as divergências restantes documentadas.

---

## Fase 3 — Núcleo de revisão (MVP usável)

Ferramentas MCP e comandos CLI equivalentes aos do CRG:

| CRG | Rust | Base existente |
|---|---|---|
| `build_or_update_graph_tool` | `build_or_update_graph` | indexer incremental |
| `detect_changes_tool` | `detect_changes` | `codegraph-parser/src/diff.rs` + `git diff` |
| `get_impact_radius_tool` | `get_impact_radius` | reverse deps (Fase 1) |
| `get_review_context_tool` | `get_review_context` | novo: nós alterados + vizinhos + testes + snippets |
| `get_minimal_context_tool` | `get_minimal_context` | novo: versão enxuta para economia de tokens |
| `query_graph_tool` | `query_graph` | callers/callees/imports/tests_for |
| `traverse_graph_tool` | `traverse_graph` | BFS no petgraph |
| `list_graph_stats_tool` | `list_graph_stats` | contagens no SQLite |
| `semantic_search_nodes_tool` | `search_nodes` | FTS5 (sem embeddings) |
| `find_large_functions_tool` | `find_large_functions` | `find_complexity_hotspots` |

Também: subcomandos `build`, `update`, `status`, `watch`, `detect-changes`, `impact`, `query`,
`search`; hook de `SessionStart` e hook de git pós-commit para update incremental; skills
`review-changes`/`review-pr`/`review-delta` adaptadas.

Critério de aceite: num repositório real, o fluxo "editar → `detect_changes` →
`get_review_context`" devolve o mesmo conjunto de arquivos/funções afetados que o CRG (via
harness da Fase 2) e mede menos tokens que ler os arquivos diretamente.

---

## Fase 4 — Análise de grafo

1. Flows: `list_flows`, `get_flow`, `get_affected_flows` (portar `flows.py`; usar a base de
   call chain).
2. Comunidades: `list_communities`, `get_community`, `get_bridge_nodes`,
   `get_architecture_overview`. Não há crate madura de Leiden: implementar Louvain/Leiden
   sobre `petgraph` com seed fixa (reprodutível) e manter o fallback por arquivo do CRG.
3. Insights: `get_hub_nodes` (já existe), `get_knowledge_gaps`,
   `get_surprising_connections`, `get_suggested_questions`.
4. `risk_index`: portar o cálculo de risco usado no review context.

---

## Fase 5 — Produto e extras

- `refactor` / `apply_refactor` (rename com preview; `refactor.py`).
- Multi-repo: `register`, `unregister`, `repos`, `list_repos`, `cross_repo_search`.
- Wiki: `generate_wiki`, `get_wiki_page`, `get_docs_section` (sem LLM por padrão).
- Visualização HTML (reaproveitar `code_review_graph/assets`), `export`.
- Instalador para as plataformas do CRG (`skills.py:PLATFORMS`): codex, claude, cursor,
  windsurf, zed, continue, opencode, gemini-cli, qwen, kiro, copilot, copilot-cli e outras;
  `install`, `uninstall`, `doctor`.
- Embeddings opcionais (feature desligada por padrão) via ONNX local, se a busca FTS5 se
  mostrar insuficiente.
- Distribuição: release com binários (Linux/macOS/Windows), Homebrew, GitHub Action
  equivalente ao `action.yml` do CRG.
- Reescrever `README.md`/`docs/`, renomear o binário/crates e definir o nome do projeto.

---

## Decisões em aberto

1. **Compatibilidade de banco**: ler e escrever o mesmo `.code-review-graph/graph.db` do CRG
   (permite migrar usuários e comparar direto), ou só importar? Recomendação: mesmo schema.
2. **Nome do binário**: manter `codegraph` ou usar `code-review-graph`/`crg`?
3. **Tiers com LSP**: manter os tiers `balanced`/`full` (mais precisos, exigem language
   servers instalados) ou só o `fast` na v1? Recomendação: manter, com `fast` como padrão.
4. **Licença do fork**: depende da resposta do upstream sobre o arquivo `LICENSE`.

## Ordem sugerida e marcos

1. Fechar a Fase 0 (commit + PR).
2. Fase 1 inteira (storage); sem ela, as fases seguintes construiriam sobre o SurrealDB.
3. Harness da Fase 2 antes de qualquer ferramenta nova, para medir regressões desde o início.
4. Fase 3 → **MVP**: binário instalável que substitui o CRG no fluxo de revisão do Claude Code.
5. Fases 4 e 5 conforme o uso real pedir.
