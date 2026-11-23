# Fase 3 — Núcleo de revisão (MVP) — plano de implementação

Referência: `PLANO.md`, seção Fase 3. Pré-requisito: Fase 2.5 completa (IDs canônicos e `line`
das arestas). Estado de partida: 8 ferramentas de grafo em `graph_tool_schemas.rs`
(`find_nodes_by_name`, `get_transitive_dependencies`, `detect_circular_dependencies`,
`trace_call_chain`, `calculate_coupling_metrics`, `get_hub_nodes`, `get_reverse_dependencies`,
`find_complexity_hotspots`). Ferramentas novas da Fase 3: 10, das quais 4 já têm base.

## Mapeamento de nomes (CRG → Rust)

| CRG | Rust (nome final) | Base no repo | Situação |
|---|---|---|---|
| `semantic_search_nodes_tool` | `search_nodes` | FTS5 em `nodes_fts` (schema v15 pronto, `reconcile.rs` já popula) | falta query + tool |
| `find_large_functions_tool` | `find_large_functions` | `get_complexity_hotspots` | renomear/filtrar por linhas |
| `list_graph_stats_tool` | `list_graph_stats` | `count_nodes_for_project` | ampliar contagens |
| `query_graph_tool` | `query_graph` | `get_reverse_dependencies`, `trace_call_chain` | novo: `callers_of`, `callees_of`, `imports_of`, `importers_of`, `tests_for` |
| `traverse_graph_tool` | `traverse_graph` | `dependencies()` (BFS interno) | expor BFS com direção e profundidade |
| `get_impact_radius_tool` | `get_impact_radius` | reverse deps | novo: dependentes transitivos de um conjunto de nós |
| `detect_changes_tool` | `detect_changes` | `codegraph-parser/src/diff.rs` | novo: `git diff` → nós tocados |
| `get_review_context_tool` | `get_review_context` | — | novo: alterados + vizinhos + testes + snippets |
| `get_minimal_context_tool` | `get_minimal_context` | — | novo: versão enxuta do anterior |
| `build_or_update_graph_tool` | `build_or_update_graph` | `ProjectIndexer::index_project` | novo wrapper MCP (lock de escrita) |

Nomes finais seguem o padrão já usado no repo (`snake_case`, verbo + substantivo). Se o usuário
preferir os nomes do CRG exatos, trocar antes da Task 1; custo de troca é baixo.

## Decisões tomadas (defaults; revisar)

1. **Ordem**: primeiro as ferramentas de consulta sobre dados já existentes (`search_nodes`,
   `list_graph_stats`, `find_large_functions`, `query_graph`, `traverse_graph`), depois as de
   revisão (`detect_changes`, `get_impact_radius`, `get_review_context`, `get_minimal_context`),
   por último `build_or_update_graph` e os subcomandos CLI. Motivo: as de revisão dependem das
   de consulta.
2. **Critério de "mesmo conjunto afetado que o CRG"**: medir com o harness de paridade em um
   repositório fixo e registrar a divergência; não exigir igualdade byte a byte de snippets.
3. **`detect_changes`**: `git diff --name-only` + `git diff -U0` contra base configurável
   (default `HEAD`, aceita `base_ref`). Linhas alteradas → nós cujo intervalo `[line, end_line]`
   cobre a linha. Arquivo sem git → erro explícito, sem fallback silencioso.
4. **Limites de saída**: reutilizar `shorten_content`/`truncate_if_oversized` do executor; não
   criar novo mecanismo de truncamento.
5. **Cache LRU**: as ferramentas novas entram no mesmo cache do executor, com chave por
   parâmetros. Invalidação já vem de `PRAGMA data_version`.
6. **Hooks e skills** (Fase 3, parte final): só depois das ferramentas verdes; sem hook de
   `SessionStart` até `build_or_update_graph` funcionar via CLI.

## Tarefas

Cada tarefa: query em `codegraph-graph/src/graph_query/`, schema + dispatch em
`codegraph-mcp-tools`, `#[tool]` em `official_server.rs` (teste
`every_graph_tool_is_exposed_over_mcp` obriga), teste em fixture pequena. Rodar
`cargo test -p codegraph-graph` e `cargo test -p codegraph-mcp-tools` a cada tarefa.

1. **`search_nodes`** (FTS5). Query `MATCH` com `bm25`, `porter unicode61`. Teste: busca por
   stem (`parsing` acha `parse`), ranking por nome antes de caminho.
2. **`list_graph_stats`**. Contagens por `kind` de nó e aresta, arquivos, linguagens, último
   `last_updated`. Teste contra fixture com contagens conhecidas.
3. **`find_large_functions`**. Filtro por `min_lines` sobre `line`/`end_line` dos nós
   `Function`/`Method`. Teste de limite.
4. **`query_graph`**. Padrões: `callers_of`, `callees_of`, `imports_of`, `importers_of`,
   `tests_for`. Parâmetro `pattern` com enum; `node_id` ou `name` (resolve via `search_nodes`).
   Alvos de texto não resolvidos são ignorados e contados em `unresolved`.
5. **`traverse_graph`**. BFS com `direction` (`out`/`in`/`both`), `edge_type`, `max_depth`
   (limite 10) e `max_nodes`. Reaproveitar `dependencies()`.
6. **`get_impact_radius`**. Entrada: `node_ids` ou `files`. Saída: dependentes transitivos por
   profundidade, arquivos afetados, testes que cobrem (`TESTED_BY`). Teste com cadeia
   A←B←C.
7. **`detect_changes`**. Ver decisão 3. Teste com repositório git temporário (criado no teste
   com `git init`, não no repo do projeto).
8. **`get_review_context`**. Une 7 + 6 + snippets das linhas alteradas (limite de linhas por
   nó). Saída ordenada por risco (`risk_index` simples: nº de dependentes + testes ausentes).
9. **`get_minimal_context`**. Mesma entrada que 8, saída só com ids, arquivos e contagens;
   teste que garante saída menor que 8 no mesmo fixture.
10. **`build_or_update_graph`** (MCP) + subcomandos CLI `build`, `update`, `status`,
    `detect-changes`, `impact`, `query`, `search`. `watch` fica para depois (já existe
    `start --watch`).
11. **Hook `SessionStart` e hook pós-commit** para `update` incremental; skills
    `review-changes`/`review-pr`/`review-delta` adaptadas.

## Critério de aceite (da Fase 3)

Repositório real: fluxo "editar → `detect_changes` → `get_review_context`" devolve o mesmo
conjunto de arquivos/funções afetados que o CRG (via harness da Fase 2) e usa menos tokens que
ler os arquivos diretamente (medir com o tamanho do JSON de saída vs. soma dos arquivos).

## Riscos

- `end_line` pode não estar gravado em todos os nós; verificar `nodes` antes da Task 3.
- `detect_changes` com caminho absoluto vs. relativo: usar `canonical_file_key` (Fase 2.5).
- Latência do `snapshot()` a cada chamada: medir após a Task 4; não otimizar antes.
