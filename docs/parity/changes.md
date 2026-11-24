# Paridade da análise de mudanças (Fase 3)

Critério de aceite da Fase 3: num repositório real, o fluxo "editar → `detect_changes` →
`get_review_context`" devolve os mesmos arquivos e funções afetados que o CRG e mede menos
tokens que ler os arquivos. Medido em 2026-11-24, binário debug no commit `3a786dc`, CRG v2.4.0
da fonte (`uv run --frozen --extra communities`).

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

## Divergências e causas

- **Lista `tests` do Rust**: `get_impact_radius` devolve `impacted`, que é o conjunto
  equivalente ao do CRG, e uma lista `tests` à parte. Essa lista inclui os testes dos nós
  impactados, ou seja, um salto de `TESTED_BY` além da profundidade. Somada, a Jaccard cai para
  0.41 (d=1) e 0.72 (d=2) no cenário 2. É uma diferença de contrato, não de grafo.
- **`affected_files` inclui os próprios arquivos alterados**; o CRG os exclui de
  `impacted_files`.
- **Cenário 1, nós impactados**:
  - Só no CRG: arquivos ligados por `IMPORTS_FROM` (`lib.rs`, `flows.rs`, `communities.rs`).
    Vem da lacuna de resolução de módulos Rust entre arquivos (Fase 2): IMPORTS_FROM resolvido
    tem Jaccard 0.50 neste repo.
  - Só no Rust: chamadas que o CRG não liga (`handle_daemon`, `DaemonManager.run_daemon`).
  - Paridade do grafo inteiro deste repo: nós Rust 0.983, CALLS 0.930, IMPORTS_FROM 0.721,
    TESTED_BY 0.951.
- **Cenário 2, d=2**: 23 nós só no CRG, quase todos `setup_method` de classes de teste. Os 3
  só no Rust vêm de uma chamada a `run_postprocess` que só o Rust resolve.
- **Flows**:
  - Cenário 1: a detecção de entry points em Rust concorda 0.826 com o CRG (este repo).
  - Cenário 3: o Rust tem flows a mais por dois motivos.
    - Ele indexa `code_review_graph/assets/d3.v7.min.js` (1066 nós, 239 flows), que o CRG
      ignora.
    - Ele detecta mais entry points em Python: 284 contra 167, Jaccard 0.555 no CRG inteiro.
      Muitos são métodos de `GraphStore` sem chamador (`get_all_nodes`, `commit`).
- **Churn**: o `detect_changes_tool` do CRG sempre soma churn (`include_churn=True` em
  `tools/review.py`). O `detect_changes` do Rust tem padrão `include_churn: false` (Decisão 8
  da Fase 4). Com o padrão, o risco geral sai 0.85 contra 1.0; com churn, os sinais batem nó a
  nó.
- **Tempo, build debug**:
  - `build` do CRG: 19 s neste repo e 57 s no CRG.
  - `build` do Rust: 26 s e 110 s.
  - `update` após a edição sintética de 2 arquivos: CRG 18 s, Rust 70 s.

## Tokens (bytes de JSON)

| cenário | arquivos tocados | `git diff` | `get_review_context` (d=2) | `get_minimal_context` |
|---|---|---|---|---|
| 1: Rust `HEAD~3` | 452 068 | 49 506 | 68 078 (6.6x menos) | 6 139 (74x menos) |
| 2: CRG sintético | 87 385 | 880 | 32 818 (2.7x menos) | 3 685 (24x menos) |
