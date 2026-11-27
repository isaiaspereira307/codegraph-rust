# Inventário de ferramentas MCP: CRG × Rust

Critério do P1 (`docs/superpowers/plans/2026-11-25-p1-paridade-e-qualidade.md`): comparar as
ferramentas MCP do Rust com as 31 do CRG e documentar as ausências. Medido em 2026-11-27 contra o
CRG `bbdda4b` (`code_review_graph/main.py`, funções `*_tool`) e o Rust `397fd24`
(`graph_tool_schemas.rs` + `build_or_update_graph` em `official_server.rs`).

Reprodução:

```bash
git -C ../code-review-graph show bbdda4b:code_review_graph/main.py \
  | grep -oE '^(async )?def [a-z_]+_tool' | sed 's/.*def //' | sort
codegraph tool list   # 27 ferramentas do executor; build_or_update_graph só no MCP
```

## Com equivalente (21 de 31)

| CRG | Rust | Nota |
|---|---|---|
| `build_or_update_graph_tool` | `build_or_update_graph` | roda `codegraph update` em processo filho |
| `detect_changes_tool` | `detect_changes` | |
| `get_impact_radius_tool` | `get_impact_radius` | segue `INHERITS`/`IMPLEMENTS` em texto (o CRG não) |
| `get_review_context_tool` | `get_review_context` | |
| `get_minimal_context_tool` | `get_minimal_context` | |
| `query_graph_tool` | `query_graph` | padrão extra `inheritors_of` resolvido por nome |
| `traverse_graph_tool` | `traverse_graph` | |
| `list_graph_stats_tool` | `list_graph_stats` | |
| `semantic_search_nodes_tool` | `search_nodes` | FTS5, sem embeddings |
| `find_large_functions_tool` | `find_large_functions` | |
| `list_flows_tool` | `list_flows` | |
| `get_flow_tool` | `get_flow` | |
| `get_affected_flows_tool` | `get_affected_flows` | |
| `list_communities_tool` | `list_communities` | Leiden próprio; ARI registrado, não igual ao igraph |
| `get_community_tool` | `get_community` | |
| `get_architecture_overview_tool` | `get_architecture_overview` | |
| `get_hub_nodes_tool` | `get_hub_nodes` | |
| `get_bridge_nodes_tool` | `get_bridge_nodes` | |
| `get_knowledge_gaps_tool` | `get_knowledge_gaps` | |
| `get_surprising_connections_tool` | `get_surprising_connections` | |
| `get_suggested_questions_tool` | `get_suggested_questions` | |

## Ausentes (10 de 31)

| CRG | Motivo | Onde fica |
|---|---|---|
| `refactor_tool` | refactor/rename com preview não portado | Fase 5 (adiado) |
| `apply_refactor_tool` | idem | Fase 5 (adiado) |
| `list_repos_tool` | multi-repo não portado | Fase 5 (adiado) |
| `cross_repo_search_tool` | idem | Fase 5 (adiado) |
| `generate_wiki_tool` | wiki não portada | Fase 5 (adiado) |
| `get_wiki_page_tool` | idem | Fase 5 (adiado) |
| `get_docs_section_tool` | idem | Fase 5 (adiado) |
| `embed_graph_tool` | embeddings removidos por desenho (Fase 0) | Fase 5, só se FTS5 for insuficiente |
| `run_postprocess_tool` | o pós-processo roda no `finalize` de todo `build`/`update`; níveis por `--skip-flows`/`--skip-postprocess` ou `CODEGRAPH_POSTPROCESS` | sem plano: `build_or_update_graph` cobre |

## Só no Rust (7)

`find_nodes_by_name`, `get_transitive_dependencies`, `get_reverse_dependencies`,
`trace_call_chain`, `detect_circular_dependencies`, `calculate_coupling_metrics`,
`find_complexity_hotspots`: ferramentas do `codegraph-rust` original reimplementadas sobre o
SQLite na Fase 1.
