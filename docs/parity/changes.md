# Paridade da análise de mudanças (Fase 3)

Critério de aceite da Fase 3: num repositório real, o fluxo "editar → `detect_changes` →
`get_review_context`" devolve os mesmos arquivos e funções afetados que o CRG e mede menos
tokens que ler os arquivos. Desde 2026-11-25 ("INHERITS no GraphQuery"), "os mesmos" quer dizer
iguais ao CRG exceto as subclasses e implementações que só o Rust alcança pela herança em texto:
o impacto do CRG não segue `INHERITS`/`IMPLEMENTS` com alvo em texto, o do Rust segue (melhoria
intencional, sem flag). Medido em 2026-11-24, binário debug no commit `3a786dc`, CRG v2.4.0
da fonte (`uv run --frozen --extra communities`); remedido em 2026-11-25 (seção "Depois de
INHERITS no GraphQuery").

## Como rodar

Cada repositório é clonado duas vezes (cada ferramenta grava o próprio
`.code-review-graph/graph.db`): `code-review-graph build --repo <clone-crg>` num clone,
`codegraph build .` no outro. Depois:

```bash
uv run --frozen --extra communities --project ../code-review-graph \
  python -I scripts/parity/changes/crg_changes.py <clone-crg> <base> crg.json <depth>
python3 scripts/parity/changes/compare_changes.py crg.json <clone-rust> <clone-crg> <base> \
  target/debug/codegraph <depth>
```

O lado CRG chama `detect_changes_func` (MCP, sempre com churn), `get_review_context` e
`GraphStore.get_impact_radius(max_nodes=1000)`. O lado Rust chama `detect_changes`
(`include_churn: true`), `get_impact_radius` (`max_nodes: 1000`) e `get_affected_flows`. Os nós
são comparados por `qualified_name` relativo à raiz.

## Cenários

1. **Este repositório (Rust), `HEAD~3`**: os 3 commits de correção da revisão da Fase 4, com
   15 arquivos.
2. **CRG (Python), edição sintética**: uma linha inserida em `compute_criticality`
   (`flows.py`) e em `_extract_keywords` (`communities.py`), commitada, seguida de `update` nas
   duas ferramentas. Esse é o fluxo literal do critério.
3. **CRG, `HEAD~1` real** (`b891b6a`): mexe em `GraphStore`, que é um hub. Com mais de 1700
   dependentes já na profundidade 1, os dois lados truncam o impacto em 1000 nós e ordenam de
   jeitos diferentes. Por isso o cenário só serve para arquivos e funções alterados.

## Resultados (Jaccard)

| conjunto | 1: Rust `HEAD~3` (d=2) | 2: sintético d=1 | 2: sintético d=2 | 3: CRG `HEAD~1` |
|---|---|---|---|---|
| arquivos alterados | 1.000 (15) | 1.000 (2) | 1.000 (2) | 1.000 (3) |
| funções alteradas | 1.000 (35) | 1.000 (2) | 1.000 (2) | 1.000 (5) |
| nós impactados (`impacted`) | 0.897 (186/182) | 0.995 (200/201) | 0.960 (714/691) | truncado |
| arquivos impactados fora dos alterados | 0.857 (41/37) | 1.000 (39) | 1.000 (201) | truncado |
| flows afetados (nome do entry) | 0.803 (69/68) | 1.000 (10) | 1.000 (10) | 0.543 (51/94) |
| risk score por função alterada, igual a ±0.001 | 31/35 | 2/2 | 2/2 | - |
| `risk_score` geral | 1.0 / 1.0 | 1.0 / 1.0 | 1.0 / 1.0 | - |

### Depois de INHERITS no GraphQuery (2026-11-25)

Binário debug em `18bcec9`, CRG da fonte em `bbdda4b`, mesmos cenários: clones deste repo em
`3a786dc` (base `HEAD~3`, d=2) e clones do CRG em `bbdda4b` com a mesma edição sintética
commitada (base `HEAD~1`). O CRG fez `build` + `update`; o Rust fez `build` direto no commit
editado (mesmo grafo final).

| conjunto | 1: Rust `HEAD~3` (d=2) antes → depois | 2: sintético d=1 antes → depois | 2: sintético d=2 antes → depois |
|---|---|---|---|
| arquivos alterados | 1.000 → 1.000 | 1.000 → 1.000 | 1.000 → 1.000 |
| funções alteradas | 1.000 → 1.000 | 1.000 → 1.000 | 1.000 → 1.000 |
| `impacted` | 0.948 (186/192) → 0.948 (186/192) | 0.995 (200/201) → 0.995 (200/201) | 0.960 (714/691) → 0.960 (714/691, 688 em comum) |
| arquivos impactados fora dos alterados | 0.976 → 0.976 | 1.000 → 1.000 | 1.000 → 1.000 |
| flows afetados | 0.880 → 0.880 | 1.000 → 1.000 | 1.000 → 1.000 |
| `risk_score` geral | 1.0 / 1.0 | 1.0 / 1.0 | 1.0 / 1.0 |

Nenhum número caiu, e os limiares da Fase 3 seguem atendidos. Os nós só no Rust são os mesmos
de antes, todos chamadas que o CRG não liga (cenário 1: os 8 listados acima; cenário 2:
`run_postprocess` e dois testes que o chamam). A herança não entra nesses cenários. No 2 os
arquivos alterados só têm funções, sem classe. No 1 há 60 tipos entre as sementes e os nós
impactados, mas nenhum tem herdeiro no repositório: os `IMPLEMENTS` Rust daqui apontam para
traits externos (`Default`, `Drop`).

**Verificação da atribuição** (edição de um arquivo que define uma base com subclasses no
repo; `get_impact_radius` por arquivo nos clones do cenário 2):

| arquivo | d | CRG | Rust | em comum | Jaccard | só Rust | só CRG |
|---|---|---|---|---|---|---|---|
| `code_review_graph/errors.py` | 1 | 62 | 39 | 38 | 0.603 | 1 | 24 |
| `code_review_graph/errors.py` | 2 | 538 | 483 | 482 | 0.894 | 1 | 56 |
| `code_review_graph/embeddings.py` | 1 | 187 | 183 | 183 | 0.979 | 0 | 4 |
| `code_review_graph/embeddings.py` | 2 | 519 | 475 | 475 | 0.915 | 0 | 44 |

- **Só no Rust**: um único nó, `graph.py::CorruptGraphDatabaseError(GraphStoreError)`, uma
  subclasse de uma base de `errors.py` alcançada pela herança em texto. Não há nó só no Rust
  sem essa explicação. Em `embeddings.py` as 5 subclasses de `EmbeddingProvider` estão no
  próprio arquivo (já são sementes), então nada muda.
- **Só no CRG** (lacunas anteriores, não ligadas à herança): na profundidade 1, 24 de 24
  (`errors.py`) e 3 de 4 (`embeddings.py`) são testes com aresta `REFERENCES` para o arquivo
  (`pytest.raises(GraphStoreError)`). O Rust não gera `REFERENCES` em Python (0 arestas; o CRG
  tem 2654). O restante é uma chamada `embeddings.LocalEmbeddingProvider(...)` após um
  `from code_review_graph import embeddings` dentro da função, que o Rust não resolve.

Ferramentas que seguem a herança em texto (usam o `Snapshot`): `get_impact_radius`,
`get_review_context` e `get_minimal_context` (pela parte de impacto e pela contagem de
`dependents`), `traverse_graph`, `query_graph` (`inheritors_of`), `get_transitive_dependencies`,
`get_reverse_dependencies`, `detect_circular_dependencies` (com `edge_type` de herança),
`calculate_coupling_metrics` e `find_complexity_hotspots` (acoplamento e `risk_score` =
complexidade × (aferente + 1)). Ferramentas que não seguem (usam o `AnalysisGraph`, igual ao
CRG): `detect_changes` (funções alteradas, `risk_score`/`risk_signals`, `review_priorities`,
flows afetados), o `risk_score` dentro de `get_review_context`, `get_hub_nodes`,
`get_bridge_nodes`, `get_knowledge_gaps`, `get_surprising_connections`,
`get_suggested_questions`, flows e comunidades. O método de biblioteca
`GraphQuery::get_hub_nodes` usa o `Snapshot`, mas a ferramenta MCP `get_hub_nodes` não o chama.

## P1 Task 11: semântica do impacto e `update` (2026-11-26)

Binário debug e release do commit da Task 11 contra o base `27cb501`; CRG `bbdda4b` da fonte
(`uv run --frozen --extra communities`). Clones do CRG em `bbdda4b`.

**Decisões** (fonte: `graph.py:get_impact_radius_sql` do CRG, onde as sementes ficam fora do
resultado e `TESTED_BY` é uma aresta da BFS):

- `get_impact_radius.tests`: só testes ao alcance de `depth`, contando o salto `TESTED_BY`.
- `affected_files`: só arquivos fora dos que contêm nós alterados.
- Entry points Python: a detecção (`analysis/flows/entry.rs`) já era a do CRG
  (`flows.py:detect_entry_points`). A diferença vinha de chamadas que o Rust não ligava, então a
  correção foi no extrator Python: receptor tipado por classe vinda de `from M import C`
  (`store: GraphStore`, `d = WatchDaemon()`) e imports no corpo da função (`from . import dsh`
  dentro de `def`). Como no `scan_declarations` do CRG, esses imports valem para o arquivo
  inteiro (depois dos imports do topo; os da própria função vêm primeiro), e um parâmetro de
  mesmo nome os esconde. `GRAPH_SHAPE_VERSION` virou
  `crg-shape-14` (`crg-shape-15` depois da revisão). Também corrigido: indexando por caminho absoluto, métodos não eram achados pelo
  nome que o catálogo reconstrói (`/abs/pkg/b.py::Thing.go`), o que derrubava
  `python_submodule_imports_and_import_bound_calls_follow_crg`.

**Entry points Python no CRG inteiro** (`parity.py compare`, flows por linguagem):

| | Rust | CRG | em comum | Jaccard entry | membros (média/mín) | CALLS |
|---|---|---|---|---|---|---|
| base `27cb501` | 268 | 167 | 162 | 0.593 | 0.935 / 0.182 | 0.900 |
| receptor por classe importada | 215 | 167 | 162 | 0.736 | 0.971 / 0.250 | 0.900 |
| + imports dentro da função | 199 | 167 | 162 | 0.794 | 0.975 / 0.250 | 0.907 |
| + escopo de arquivo e sombra por parâmetro (revisão) | 199 | 167 | 162 | 0.794 | 0.975 / 0.250 | 0.907 |

O alvo do plano (diferença ≤ 5%) **não foi atingido**. Os 37 entry points só no Rust:

- 16 receptores cujo tipo o Rust não infere: retorno anotado de função
  (`store, root = _get_store(...)`) e atributo `self.x` atribuído com construtor
  (`self._config_watcher = ConfigWatcher(...)`). O CRG porta isso em
  `_apply_python_receiver_evidence`.
- 16 funções aninhadas homônimas: o Rust grava `CodeParser.visit@L10058`, o CRG
  `CodeParser.visit:S<hash>`; são nós diferentes nos dois grafos, não uma diferença de detecção.
- 5 outros: chamada recursiva (`CodeParser.visit` → ele mesmo), propriedades e testes que o CRG
  liga por evidência de tipo (`LocalEmbeddingProvider.dimension`).

Os 5 só no CRG são 2 nomes aninhados (`:S<hash>`) e 3 métodos que o Rust liga a um chamador que o
CRG não liga.

**Cenários** (mesmo comando de "Como rodar"):

| conjunto | 1: Rust `HEAD~3` (d=2) | 2: sintético d=1 | 2: sintético d=2 | 3: CRG `HEAD~1` |
|---|---|---|---|---|
| arquivos / funções alterados | 1.000 / 1.000 | 1.000 / 1.000 | 1.000 / 1.000 | 1.000 / 1.000 |
| `impacted` | 0.948 (186/192) | 0.995 (200/201) | 0.957 (714/695) | truncado |
| `impacted` + `tests` | 0.748 (186/244) | 0.995 | 0.957 | - |
| arquivos impactados fora dos alterados | 0.976 | 1.000 | 1.000 | 0.670 (truncado) |
| flows afetados | 0.880 | 1.000 | 1.000 | 0.895 |
| risco por função igual | 32/35 | 2/2 | 2/2 | 5/5 |

O cenário 2 foi refeito com outra edição sintética (`_parity_edit = "s2"` no início das duas
funções); o binário base no mesmo cenário dá 0.995 e 0.960 (688 em comum, 26 só no CRG, 3 só no
Rust). Com a Task 11, d=2 fica 0.957: 25 só no CRG e 6 só no Rust. Os 6 só no Rust são
`tools/build.py::run_postprocess` e testes que o chamam. O `run_postprocess` chama
`_trace_flows` (de `from code_review_graph.flows import trace_flows as _trace_flows` dentro da
função), que o CRG deixa como texto e o Rust liga (antes pelo fuzzy léxico, agora pelo import).
Os 3 testes novos chegam a ele por `import code_review_graph.tools.build as build_module` dentro
do teste, receptor que o CRG também liga. A queda de 0.003 vem toda desse nó; sem ele o Rust
ganhou um nó em comum.

No cenário 1, a linha `impacted` + `tests` ainda soma 52 testes Rust (`#[test]`) que o CRG não
alcança: ligações `TESTED_BY` que só o Rust grava neste repo (TESTED_BY 0.974 no grafo inteiro).

**`update` após edição sintética de 2 arquivos** (CRG, 3 repetições, cada uma com edição nova;
segundos):

| binário | CRG | Rust debug | Rust release |
|---|---|---|---|
| base `27cb501` (medianas) | 59.8 (24.6–63.9) | 105.5 | 9.9 |
| Task 11 (medianas) | 17.2 (16.0–51.5) | 99.0 | 9.8 |

O tempo do CRG varia de 16 s a 64 s entre execuções nesta máquina (WSL). O alvo de release
(≤ 1,2× o CRG, ou seja, ≤ 20,6 s pela mediana menor) foi atingido. O alvo "debug ≤ 2× o release"
foi descartado: debug é `opt-level = 0` e não é distribuído. O pós-processo não domina: num
`index --stats-json` release, `resolution_exact_lexical` leva 11,6 s de 14,8 s, porque re-resolve
as ~92k arestas do projeto a cada `update` (2,4 s em `Calls`, 1,2 s em 2752 arestas `Defines` no
fuzzy léxico, medido com instrumentação temporária). Resolver só o que mudou fica para depois.

## Divergências e causas

- **Lista `tests` do Rust** (decidido na Task 11 do P1, 2026-11-26): `get_impact_radius`
  devolve `impacted`, o conjunto equivalente ao do CRG, e uma lista `tests` à parte. Até a Task
  11 essa lista ia um salto de `TESTED_BY` além da profundidade (somada, Jaccard 0.41 em d=1 e
  0.72 em d=2 no cenário 2). Agora o salto conta como nível, como na BFS do CRG
  (`graph.py:get_impact_radius_sql`, onde `TESTED_BY` é aresta da caminhada): só entram testes
  de nós alterados e de impactados acima do último nível. No cenário 2 a soma passou a 0.995
  (d=1) e 0.957 (d=2), igual a `impacted`.
- **`affected_files`** (decidido na Task 11): passou a excluir os arquivos que contêm nós
  alterados, como o `impacted_files` do CRG (as sementes ficam fora do resultado). Os arquivos
  alterados continuam em `changed_files` (`get_review_context`) e em `changed`.
- **Cenário 1, nós impactados**:
  - Só no CRG: arquivos ligados por `IMPORTS_FROM` (`lib.rs`, `flows.rs`, `communities.rs`).
    Vem da lacuna de resolução de módulos Rust entre arquivos (Fase 2): IMPORTS_FROM resolvido
    tem Jaccard 0.50 neste repo.
  - Só no Rust: chamadas que o CRG não liga (`handle_daemon`, `DaemonManager.run_daemon`).
  - Paridade do grafo inteiro deste repo: nós Rust 0.983, CALLS 0.930, IMPORTS_FROM 0.721,
    TESTED_BY 0.951.
  - **Depois da resolução de módulos Rust** (2026-11-24, binário debug em `3840ab1`, CRG da
    fonte em `bbdda4b`, mesmo cenário: clones em `3a786dc`, base `HEAD~3`, d=2): `impacted`
    0.897 → 0.948 (186/192, 184 em comum); arquivos impactados fora dos alterados 0.857 → 0.976
    (41/42); flows afetados 0.803 → 0.880 (69/72); arquivos e funções alterados seguem 1.000;
    risco 1.0 / 1.0. Os `IMPORTS_FROM` que faltavam agora existem nos dois lados. Só no CRG (2):
    `GraphQuery.node_coverage` e `GraphQuery.get_impact_radius`. Só no Rust (8):
    `handle_daemon`, `handle_daemon_start`, `handle_estimate`, `handle_index`,
    `DaemonManager.run_daemon`/`start_background`, `tool_cli.rs::execute`/`run`, chamadas que o
    CRG não liga. Grafo inteiro deste repo no HEAD `3840ab1`: IMPORTS_FROM 0.991 (resolvido
    1.000), CALLS 0.964, TESTED_BY 0.974.
- **Cenário 2, d=2**: 23 nós só no CRG, quase todos `setup_method` de classes de teste. Os 3
  só no Rust vêm de uma chamada a `run_postprocess` que só o Rust resolve.
- **Flows**:
  - Cenário 1: a detecção de entry points em Rust concorda 0.826 com o CRG (este repo).
  - Cenário 3: o Rust tem flows a mais por dois motivos.
    - Ele indexava `code_review_graph/assets/d3.v7.min.js` (1066 nós, 239 flows), que o CRG
      ignora. Corrigido em 2026-11-24: `*.min.js`/`.min.css`/`.min.json` saem da coleta
      (`is_minified_asset`), e os flows do Rust no CRG caíram de 542 para 303 (CRG: 186).
    - Ele detecta mais entry points em Python: 284 contra 167, Jaccard 0.555 no CRG inteiro.
      Muitos são métodos de `GraphStore` sem chamador (`get_all_nodes`, `commit`). Atualizado na
      Task 11 do P1 (seção abaixo): 199 contra 167, Jaccard 0.794.
- **Churn** (corrigido em 2026-11-24): o `detect_changes_tool` do CRG sempre soma churn
  (`include_churn=True` em `tools/review.py`); o `detect_changes` do Rust passou a ter o mesmo
  padrão (`include_churn: true`). Antes, com `false`, o risco geral saía 0.85 contra 1.0.
- **Tempo, build debug**:
  - `build` do CRG: 19 s neste repo e 57 s no CRG.
  - `build` do Rust: 26 s e 110 s.
  - `update` após a edição sintética de 2 arquivos: CRG 18 s, Rust 70 s.

## Tokens (bytes de JSON)

| cenário | arquivos tocados | `git diff` | `get_review_context` (d=2) | `get_minimal_context` |
|---|---|---|---|---|
| 1: Rust `HEAD~3` | 452 068 | 49 506 | 68 078 (6.6x menos) | 6 139 (74x menos) |
| 2: CRG sintético | 87 385 | 880 | 32 818 (2.7x menos) | 3 685 (24x menos) |
