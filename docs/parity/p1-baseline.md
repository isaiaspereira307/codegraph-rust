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
| typescript-js | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | n/a | n/a | 1.000 | 1.000 |
| go-package | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | n/a | TESTED_BY 1.000 | 1.000 | 1.000 |
| java-cross-file | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | INHERITS 1.000 / INJECTS 1.000 | n/a | 1.000 | 1.000 |
| java-framework | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | n/a | n/a | 1.000 | 1.000 |

`java-cross-file` também mede CONSUMES 1.000; `java-framework` mede DEPENDS_ON_CONFIG,
HANDLES, PUBLISHES, TEMPORAL_STUB e TRIGGERS em 1.000. Detalhes e decisões de escopo em
[`p1-task8-java-cross-file.md`](p1-task8-java-cross-file.md).

CONTAINS menor nas sondas antigas vem da hierarquia de diretórios que só o Rust grava. Em
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

Task 11 (2026-11-26): o piso do cenário 2 d=2 baixou de 0.960 para 0.957. A diferença são testes
que chegam a `tools/build.py::run_postprocess`, nó que só o Rust alcança (o CRG deixa como texto
o `from M import f as _f` dentro da função); eles chegam lá por um `import ... as m` local que o
CRG também liga como receptor. Detalhes em [`changes.md`](changes.md#p1-task-11-semântica-do-impacto-e-update-2026-11-26).

## Tempo de `update` (2 arquivos, debug)

| ferramenta | segundos |
|---|---|
| CRG `update` | 24,89 |
| Rust `codegraph update .` | 77,40 |

Medição da mesma rodada do snapshot (binário debug, pós-processo completo). Os números
históricos de `docs/parity/changes.md` são CRG 18 s e Rust 70 s. O alvo do P1 é medir release e
debug com 3 repetições na Task 11; estes valores são apenas o registro da linha de base.

## Atualizações P1

- Task 3: receptores tipados portados (`service: AuthService`, `svc = AuthService()`,
  `q.lib.f()`, `q.Thing().go()`). Corpus e sonda python: CALLS e resolvido 1.000; ARI de corpus
  0.403 -> 0.478; entry points/membros do corpus 1.000; nós/IMPORTS_FROM inalterados em 1.000.
  Snapshot completo `/tmp/opencode/parity-p1-t3`: 82 pisos OK. Medição em
  [`p1-task3-python-receivers.md`](p1-task3-python-receivers.md). `REFERENCES` continua **desligado no
  `fast`**: a simulação com extração/filtro ligados (sem LSP, porque `full`/`balanced` exigem
  `pyright-langserver` ausente) leva REFERENCES a 1.000 e ARI a 1.000, mas cresce o grafo Rust
  7,9% no corpus (63 -> 68 arestas), acima do limite de 5% do plano. Detalhes em
  [`baseline.md`](baseline.md).
- Task 4: shims `sys.modules` e reexportações portados em `ModuleIndex`/`SymbolCatalog`.
  Sonda `python-imports` (agora com `billing_lib`/`engine`/shims): CALLS 0.720 → 0.909
  (resolvido 0.941), IMPORTS_FROM 1.000, nós 1.000, CONTAINS 0.920. Snapshot completo
  `/tmp/opencode/parity-p1-t4`: 82 pisos OK. Medição em
  [`p1-task4-python-reexports.md`](p1-task4-python-reexports.md).
- Task 5: `super::` limitado ao raiz do crate, fixtures de reexport/`#[path]`/receptor tipado no
  probe `rust-imports` e `reachable` com fatia emprestada. Sonda: CALLS 0.867 → 1.000,
  IMPORTS_FROM/IMPLEMENTS/nós 1.000; corpus Rust 1.000 em todas as métricas. Snapshot completo
  `/tmp/opencode/parity-p1-t5`: 82 pisos OK. A/B release (`build . --skip-postprocess`, 10 pares)
  mediana 16,46 s → 15,51 s. Medição em
  [`p1-task5-rust-resolution.md`](p1-task5-rust-resolution.md).
- Task 6: probe `typescript-js` com JSX/`memo`/`forwardRef`, `require`/`import()`, alias de
  workspace/npm, funções atribuídas a membro e guarda morta. Sonda: nós 0.714/0.333 → 1.000 em
  JS/TSX, CALLS 0.200 → 1.000, CONTAINS 0.471 → 1.000, IMPORTS_FROM 0.500 → 1.000; corpus TS
  segue em 1.000. Nós sintéticos de diretório saíram da indexação (o CRG não os emite). Snapshot
  completo `/tmp/opencode/parity-p1-t6`: 86 pisos OK (82 + 4 novos pisos do probe). Medição em
  [`p1-task6-ts-js.md`](p1-task6-ts-js.md), incluindo a divergência conhecida de receptor tipado.
- Task 7: probe `go-package` com módulo, `replace`, `vendor` e `_test.go`. Sonda: CALLS
  0.333 → 1.000 e IMPORTS_FROM 0.333 → 1.000; nós/CONTAINS/TESTED_BY e corpus Go seguem em
  1.000. Import Go passa a mirar o diretório do pacote com `import_scope=package`, sem nó
  sintético; chamadas ligam-se por evidência única de import. Snapshot completo
  `/tmp/opencode/parity-p1-t7`: 91 pisos OK (86 + 5 do probe). Medição em
  [`p1-task7-go-packages.md`](p1-task7-go-packages.md), com o ruling R8 sobre o ramo ambíguo
  que não existe mais no CRG congelado.
- Task 12: `flows.id` estável. `store_flows` reconcilia contra as linhas gravadas pela chave
  `qualified_name` do entry point e só escreve o que mudou (traço e criticalidade seguem
  completos). Neste repo, 3 builds debug seguidos: 0 de 355 ids iguais antes, 356 de 356 depois;
  `flows_store_s` mediana 0,0131 s → 0,0037 s, `flows_s` (traço) inalterado. Snapshot completo:
  151 pisos OK. Detalhes em [`p1-task12-flows.md`](p1-task12-flows.md).
- Task 13: arestas de import gravam `extra.import_context` (Python, JS/TS, Ruby) e
  `import_cycles` ignora arquivos de nós `extra.virtual`. Neste repo, Rust e CRG têm os mesmos
  320 pares `File → File`, 318 após o filtro e 15 em 4 ciclos. Detalhes em
  [`p1-task13.md`](p1-task13.md).
- Task 14: desempates das análises (entry points/caminho dos flows, hubs, pontes, lacunas,
  conexões surpreendentes) pela ordem canônica relativa ao repo, como as comunidades
  (`161390f`). Escala a 12k/25k/50k nós em [`scale.md`](scale.md): comunidades e memória
  lineares, flows superlineares pelo número de memberships (igual ao CRG). Herança no
  `AnalysisGraph` medida e recusada, `OVERRIDES` descartado:
  [`analysis-heritage.md`](analysis-heritage.md).

## Fechamento (Task 15, 2026-11-27)

Snapshot final com o binário debug de `397fd24` e CRG `bbdda4b`:

```bash
bash scripts/parity/p1_snapshot.sh <scratch>/p1-final
python3 scripts/parity/check_p1.py <scratch>/p1-final   # P1 baseline OK: 151 floors
```

Pisos elevados ao valor medido (R21; o Leiden é determinístico desde a Task 14a):

| métrica | piso antigo | piso novo |
|---|---|---|
| corpus python CALLS / resolvido | 0.923 / 0.833 | 1.000 / 1.000 |
| corpus python entry points / membros (média, mín.) | 0.750 / 0.833, 0.500 | 1.000 / 1.000, 1.000 |
| corpus python ARI | 0.403 | 1.000 |
| sonda python CONTAINS | 0.902 | 0.958 |
| sonda python-imports CALLS / resolvido / CONTAINS | 0.684 / 0.786 / 0.909 | 0.909 / 0.941 / 1.000 |
| sonda rust-imports CONTAINS | 0.737 | 1.000 |
| risco por função, cenário 1 | 31/35 | 32/35 |

Critério de aceite do P1:

| métrica | alvo | medido | status |
|---|---|---|---|
| 5 linguagens prioritárias | sem queda; python CALLS ≥ 0.95 | todas 1.000 em nós/CALLS/IMPORTS_FROM/ARI | ✅ |
| 7 linguagens restantes | nós ≥ 0.95, IMPORTS_FROM ≥ 0.90, CALLS ≥ 0.80 | 1.000 nas 7 sondas; INHERITS Kotlin 0.167 e CONTAINS Dart 0.833 ([detalhes](p1-task10-grammars.md)) | ✅ |
| `qualified_name` nos 3 modos de caminho | idêntico | idêntico (Task 2) | ✅ |
| cenários de mudança | sem queda; risco ≥ 34/35; entry points Δ ≤ 5% | impacted 0.948 / 0.995 / 0.957 (d=2 rebaixado na Task 11, decisão documentada); risco 32/35; entry points Python no repo do CRG 199 contra 167 | ❌ risco e entry points ([changes.md](changes.md)) |
| `update` de 2 arquivos | release ≤ 1,2× CRG; debug ≤ 2× release | release atingido (Task 11); debug 66,3 s contra CRG 15,1 s nesta rodada; alvo de debug descartado (R11) | ✅ release, debug descartado |
| flows | ID estável; sem queda | 356/356 IDs estáveis (Task 12) | ✅ |
| escala | 50k nós medidos | [scale.md](scale.md) | ✅ |
| ferramentas MCP | inventário contra as 31 do CRG | 21 com equivalente, 10 ausentes (Fase 5 ou removidas por desenho), 7 só no Rust: [mcp-inventory.md](mcp-inventory.md) | ✅ |

Ficam abertos, sem plano: risco 32/35 (a causa é CALLS/flows/comunidades, não fallbacks; R17),
entry points Python a mais no repo do CRG (receptor por anotação de retorno/atributo `self.x`, nomes de
`def` aninhado), `REFERENCES` desligado no `fast`, e o `update` dominado por
`resolution_exact_lexical` (re-resolve todas as arestas a cada execução).

Verificação do fechamento: `cargo fmt --all -- --check` limpo; `cargo test -p codegraph-parser
-p codegraph-mcp -p codegraph-graph` e `cargo test -p codegraph-mcp --test e2e_mcp_tools` verdes.
`cargo clippy -p codegraph-parser -p codegraph-mcp -p codegraph-graph --all-targets -- -D warnings`
falha nos 23 diagnósticos antigos de `codegraph-core`; sem `-D warnings`, os crates
`codegraph-parser`, `codegraph-mcp`, `codegraph-mcp-core` e `codegraph-graph` têm 92 avisos
(alguns em extratores novos da P1, como `ruby.rs`), sem limpeza planejada.
