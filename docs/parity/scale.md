# Escala do build e do pós-processo (Task 14b)

Repo Python sintético de `scripts/parity/gen_synthetic.py` (determinístico, seed 14): por módulo
uma classe com 8 métodos e 10 funções (~20 nós), pacotes de 50 módulos, cada função chama até
duas do próprio módulo e uma importada de outro módulo do mesmo pacote ou (1 em 10) de um
pacote anterior; 1 em 10 módulos tem um módulo de teste. Tier `fast`.

Binário release de `161390f` (`cargo build --release -p codegraph-mcp-server --bin codegraph
--features full`), WSL2. Cada linha: `rm -rf .code-review-graph && RUST_LOG=info /usr/bin/time
-v codegraph build .`, 3 repetições, mediana (min–max). `flows_s` inclui carregar o
`AnalysisGraph`; os três tempos vêm do log `analysis stored`.

```bash
python3 scripts/parity/gen_synthetic.py <dir> --modules 600    # 1250, 2500
```

| módulos | nós | arestas | flows | memberships | build (wall) | `flows_s` | `flows_store_s` | `communities_s` | RSS máx |
|---|---|---|---|---|---|---|---|---|---|
| 600 | 12 252 | 42 911 | 605 | 1 780 061 | 27,4 s (24,7–31,1) | 3,8 (3,6–4,7) | 6,7 (6,7–7,2) | 0,09 | 281 MB |
| 1 250 | 25 525 | 89 489 | 1 267 | 6 826 345 | 131,5 s (127,2–133,3) | 27,3 (22,7–30,2) | 47,1 (44,6–50,7) | 0,24 | 497 MB |
| 2 500 | 51 050 | 178 913 | 2 547 | 21 811 972 | 475,9 s (407,0–646,5) | 89,1 (80,9–112,2) | 219,1 (159,5–227,4) | 0,50 | 1 125 MB |

Fora do pós-processo, a resolução de relações do indexador ("Resolved N of M relationships ...
in") levou 9,6 s, 40,1 s e 129,9 s (run 1).

## Crescimento

- Comunidades (Leiden): linear (0,09 → 0,24 → 0,50 s para 2× → 4× nós).
- Memória: ~linear (281 → 497 → 1 125 MB).
- Flows: **pior que linear**. Cada entry point (≈ 1 por 20 nós) gera um BFS de até 15 níveis
  por `CALLS`, e o alcance médio cresce com o repo (2 942 → 5 388 → 8 564 nós por flow), então
  as memberships crescem ~n^1,8 (1,8 M → 6,8 M → 21,8 M). `flows_s` acompanha as memberships
  (~4 s por milhão a partir de 25k nós); `flows_store_s` cresce um pouco mais que elas
  (3,8 → 6,9 → 10,0 s por milhão), custo de manter o índice de `flow_memberships` em tabelas de
  dezenas de milhões de linhas.
- O CRG tem o mesmo algoritmo e a mesma saída: no repo de 600 módulos ele grava os mesmos 605
  flows e 1 780 061 memberships (build CRG 55,6 s, RSS 249 MB; Rust 27,4 s, 281 MB). O
  crescimento é do modelo de flows (um flow por entry, todas as memberships gravadas), não do
  porte.
- A resolução de relações do indexador também cresce mais que linear (×4,2 e ×3,2 por
  dobra); está fora do escopo desta task e fica registrada.

## Decisão

Sem otimização nesta task: o crescimento superlinear está no número de memberships que o modelo
do CRG exige, e reduzi-lo (limitar alcance, não gravar memberships, gravação em lote ou sem
índice) muda a saída ou é uma otimização grande. Pontos para quem retomar: `reconcile_flows`
insere uma membership por `execute` (lote multi-linha reduziria o custo por linha), e a
resolução de relações do indexador. Repo sintético é pessimista: o grafo de `CALLS` é denso e
alcança milhares de nós por entry; neste repositório `flows_s` fica abaixo de 1 s mesmo em debug (`p1-task12-flows.md`).
