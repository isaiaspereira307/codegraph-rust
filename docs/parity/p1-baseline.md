# P1 — baseline congelado

Linha de base do plano `docs/superpowers/plans/2026-11-25-p1-paridade-e-qualidade.md`.
Medida em 2026-11-25 com o binário debug construído em `8ced807` e CRG `bbdda4b` da fonte
(`uv run --frozen --extra communities`). Reprodução:

```bash
bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1
python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1
python3 scripts/parity/check_p1.py --print /tmp/opencode/parity-p1  # todos os valores medidos
```

`scripts/parity/check_p1.py` contém os **limites** (pisos) derivados destes números; o
`docs/parity/baseline.md` continua como log histórico das fases anteriores. Onde a medição atual
é melhor que a histórica, o piso fica no valor histórico publicado (ex.: ARI de python 0.403,
risco 31/35) para não transformar uma oscilação do Leiden numa falsa regressão.

## Corpus por linguagem

| linguagem | nós | CALLS | resolvido | linha | IMPORTS_FROM | CONTAINS | herança | TESTED_BY | entry points | membros | ARI |
|---|---|---|---|---|---|---|---|---|---|---|---|
| python | 1.000 | 0.923 | 0.833 | 1.000 | 1.000 | 1.000 | INHERITS 1.000 | 1.000 | 0.750 (3/4) | 0.833 / 0.500 | 0.563 |
| typescript | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | INHERITS 1.000 | 1.000 | 1.000 (2/2) | 1.000 / 1.000 | 1.000 |
| go | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | n/a | n/a | 1.000 (13/13) | 1.000 / 1.000 | 1.000 |
| rust | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | IMPLEMENTS 1.000 | 1.000 | 1.000 (5/5) | 1.000 / 1.000 | 1.000 |
| java | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | INHERITS 1.000 / INJECTS 1.000 | n/a | 1.000 (6/6) | 1.000 / 1.000 | 1.000 |

`REFERENCES` é 0 no tier `fast` em python/typescript (5 e 7 arestas só no CRG). O ARI de python
nesta rodada (0.563) difere dos 0.403/0.909 registrados em `baseline.md`; a causa documentada é
a variação do Leiden nos dois lados (o parser python não mudou). Piso: 0.403.

## Sondas

| sonda | nós | CALLS | resolvido | linha | IMPORTS_FROM | CONTAINS | herança | TESTED_BY | entry points | ARI |
|---|---|---|---|---|---|---|---|---|---|---|
| python | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.902 | INHERITS 1.000 | 1.000 | 1.000 (8/8) | 1.000 |
| python-imports | 1.000 | 0.684 | 0.786 | 0.846 | 1.000 | 0.909 | n/a | n/a | 1.000 (2/2) | 0.821 |
| rust-imports | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.737 | IMPLEMENTS 1.000 | n/a | 1.000 | 0.611 |
| inheritance | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.816 | INHERITS 1.000 | 1.000 | 1.000 | 1.000 |

CONTAINS menor nas sondas vem da hierarquia de diretórios que só o Rust grava. Em
`python-imports` faltam os caminhos de módulo pontuado (`q.lib.f()`) e o construtor de módulo
(`mod.Thing().go()`); em `python` a única chamada que falta é a de receptor tipado por
anotação (`process_request` → `AuthService.authenticate`).

`inheritance` (25 arquivos, Jaccard somado Σ∩/Σ∪): `inheritors_of` go 1.000, java 0.250,
py 0.250, ts 0.500; impacto d1 go 0.000, java 0.000, py 0.500, ts 0.500; impacto d2 vai
em `metrics.json`. O impacto do CRG não segue herança em texto; o Rust segue (decisão da Fase 4).

## Análise de mudanças

| cenário | arquivos alterados | funções alteradas | `impacted` | arquivos impactados | flows afetados | risco por função |
|---|---|---|---|---|---|---|
| 1: repo `3a786dc`, base `HEAD~3`, d=2 | 1.000 (15) | 1.000 (35) | 0.948 (186/192) | 0.976 | 0.880 | 32/35 |
| 2: CRG sintético, d=1 | 1.000 (2) | 1.000 (2) | 0.995 (200/201) | 1.000 | 1.000 | 2/2 |
| 2: CRG sintético, d=2 | 1.000 (2) | 1.000 (2) | 0.960 (714/691) | 1.000 | 1.000 | 2/2 |
| 3: CRG `b891b6a`, base `HEAD~1`, d=2 | 1.000 (3) | 1.000 (5) | truncado | truncado | 0.543 | 3/5 |

O risco 32/35 é uma melhoria sobre os 31/35 históricos; o piso do detector continua 31/35.
O cenário 3 só é comparável em arquivos/funções alterados (ambos truncam o impacto em 1000 nós).

## Tempo de `update` (2 arquivos, debug)

| ferramenta | segundos |
|---|---|
| CRG `update` | 24,89 |
| Rust `codegraph update .` | 77,40 |

Medição da mesma rodada do snapshot (binário debug, pós-processo completo). Os números
históricos de `docs/parity/changes.md` são CRG 18 s e Rust 70 s. O alvo do P1 é medir release e
debug com 3 repetições na Task 11; estes valores são apenas o registro da linha de base.
