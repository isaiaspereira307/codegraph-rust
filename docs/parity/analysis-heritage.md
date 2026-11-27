# Herança no `AnalysisGraph` e `OVERRIDES` (Task 14b)

Decisão final: o `AnalysisGraph` (flows, comunidades, `risk_scores`, `review_priorities`, hubs,
pontes, lacunas, conexões surpreendentes) **continua sem ligar** alvos de texto de
`INHERITS`/`IMPLEMENTS`; só o `Snapshot` do `GraphQuery` os liga por nome. `OVERRIDES` é
**descartado**. Nenhuma flag foi adicionada.

## A divisão `Snapshot` vs `AnalysisGraph`

- O indexador grava a aresta de herança com o texto da base como alvo (`Base`,
  `unittest.TestCase`), como o CRG.
- `Snapshot` (`crates/codegraph-graph/src/graph_query/snapshot.rs`) liga esse texto a um nó de
  tipo por `graph_query/heritage.rs` (mesmo arquivo, depois arquivos importados, depois o repo
  inteiro; ambíguo ou externo não vira aresta). É o que faz `inheritors_of`, o raio de impacto e o
  acoplamento alcançarem subclasses (plano `2026-11-25-inherits-graphquery`).
- `AnalysisGraph` (`crates/codegraph-graph/src/analysis/mod.rs`) carrega as arestas como estão
  gravadas. Um alvo de texto não é nó: comunidades o ignoram, pontes o tratam como vértice de
  texto, flows só seguem `CALLS`. É o comportamento das análises do CRG (`communities.py`,
  `analysis.py`, `flows.py`), que também leem `edges` cruas.

## Medição

Experimento descartável (não commitado): em `AnalysisGraph::load`, com
`CODEGRAPH_EXPERIMENT_ANALYSIS_HERITAGE=1`, cada `INHERITS`/`IMPLEMENTS` cujo alvo não é nó foi
reescrito para o nó que `resolve_heritage` escolhe (mesma regra do `Snapshot`; só vínculos
únicos). Binário debug de `161390f` + o experimento; snapshot P1 completo
(`scripts/parity/p1_snapshot.sh`, que inclui o harness `scripts/parity/changes/` e a sonda
`inheritance`) com a variável ligada, comparado ao snapshot do mesmo commit sem ela.

| métrica (Rust vs CRG) | sem ligar herança | ligando herança |
|---|---|---|
| `corpus.python.ari` | 1.000 | 0.626 |
| `corpus.java.ari` | 1.000 | 0.798 |
| `probes.inheritance.ari` | 1.000 | 0.363 |
| `probes.java-cross-file.ari` | 1.000 | 0.907 |
| `probes.dart-imports.ari` | 1.000 | 0.571 |
| `changes.rust-head3.risk_equal` | 32/35 | 31/34 |
| `changes.rust-head3.changed_functions` | 1.000 | 0.944 |
| demais métricas (entry points, membros de flows, affected flows, impacto) | — | iguais |

`check_p1.py` no snapshot com herança: **3 pisos caem** (`corpus.java.ari`,
`probes.java-cross-file.ari`, `changes.rust-head3.changed_functions`).

- Flows e affected flows não mudam: o traço só segue `CALLS`.
- Comunidades pioram em todas as sondas com herança: as arestas novas (peso 0,8/0,7) puxam
  subclasses para a comunidade da base, o que o Leiden do CRG não vê.
- Risco: o único caso perdido em `rust-head3` é uma função cujo nome qualificado o lado CRG traz
  truncado (`…minimal_hubs_keep_a_chainable_qualified_nam`), que sai de `changed_functions` e do
  denominador; nas 34 comuns o número de riscos iguais é o mesmo (31 = 32 − 1, se a perdida era
  igual). Sem ganho de paridade em risco.
- Pontes e hubs (top 10, sonda `inheritance`, `get_bridge_nodes`/`get_hub_nodes` contra
  `find_bridge_nodes`/`find_hub_nodes` do CRG): sem herança, pontes 10/10 na mesma ordem e hubs
  9/10; com herança, pontes 9/10 em outra ordem e hubs 7/10 (as bases `Shape` de Java/Go/TS
  sobem por grau).
- Determinismo: o vínculo usa `resolve_heritage`, cuja ordem de candidatos é (arquivo, nome
  qualificado), independente do work-dir; o experimento não quebraria determinismo, mas a
  paridade piora.

Critério da Task 14 ("se melhorar paridade sem quebrar determinismo, implementar atrás de flag"):
não melhora, então a divisão fica como decisão final.

## `OVERRIDES`

No CRG congelado (`bbdda4b`) nenhum extrator emite `OVERRIDES` (`grep` em `parser.py` e nos
resolvers: 0 ocorrências). O tipo só aparece em `constants.py`, em `IMPACT_EDGE_WEIGHTS` (0,9) e
`IMPACT_EDGE_DIRECTIONS` (incoming), e num teste de `get_impact_radius_sql` que insere a aresta à
mão. Como nenhum banco do CRG contém `OVERRIDES`, portar o tipo (extrator, peso, direção) não
muda nenhuma saída comparável e criaria arestas que o CRG não tem. Descartado; se o CRG passar a
emiti-lo, o porte é o peso/direção no impacto mais o extrator correspondente.
