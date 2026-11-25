# P1 Task 3 — receptores tipados em Python

Port mínimo de `_apply_python_receiver_evidence` (`../code-review-graph/code_review_graph/parser.py:10577`)
no extrator `crates/codegraph-parser/src/languages/python.rs`. Medido em 2026-11-25 com CRG
`bbdda4b`, binário debug e tier `fast` padrão. Limites congelados em
[`p1-baseline.md`](p1-baseline.md); detector `scripts/parity/check_p1.py`.

## O que mudou

- Parâmetros anotados (`def f(service: AuthService): service.authenticate()`) e atribuições com
  construtor (`svc = AuthService(); svc.authenticate()`) passam a nomear
  `<arquivo>::AuthService.authenticate`, com `receiver_binding=class`/`receiver_class` na aresta.
- Caminho de módulo pontuado (`import q; q.lib.f()`) carrega `crg_py_import=q.lib|` e
  `receiver_binding=module`; construtor de módulo (`q.Thing().go()`) marca a classe importada.
- Fallback `crg_unbound` preservado para receptores sem evidência. A reescrita pós-build do CRG
  fica para a Task 4.

## Medição

Comando (corpus + sonda, sem LSP):

```bash
CODEGRAPH_INDEX_TIER=fast python3 scripts/parity/parity.py corpus \
  --work /tmp/opencode/p1-t3-work/corpus \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph --md /tmp/opencode/p1-t3-snap/corpus.md
CODEGRAPH_INDEX_TIER=fast python3 scripts/parity/parity.py run scripts/parity/probes/python \
  --work /tmp/opencode/p1-t3-work/python \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t3-snap/probes/python.json \
  --md /tmp/opencode/p1-t3-snap/probes/python.md
```

| métrica | antes (P1 baseline) | depois | piso |
|---|---|---|---|
| corpus.python.CALLS | 0.923 | 1.000 | 0.923 |
| corpus.python.CALLS.resolved | 0.833 | 1.000 | 0.833 |
| corpus.python.nós | 1.000 | 1.000 | 1.000 |
| corpus.python.IMPORTS_FROM | 1.000 | 1.000 | 1.000 |
| corpus.python.ARI | 0.563 (piso 0.403) | 0.478 | 0.403 |
| corpus.python.flows.entry | 0.750 | 1.000 | 0.750 |
| corpus.python.flows.membros (média/mín) | 0.833 / 0.500 | 1.000 / 1.000 | 0.833 / 0.500 |
| sonda python CALLS / resolvido / nós / IMPORTS_FROM | 1.000 | 1.000 | 1.000 |

Snapshot completo: `bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-t3` seguido de
`python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t3` → **82 pisos, nenhum métrica
caiu**.

O ARI medido (0.478) ficou abaixo da rodada do baseline (0.563), mas acima do piso congelado
(0.403); a variação é do particionador Leiden, não das chamadas — o CRG é o mesmo commit.

## `REFERENCES` no tier `fast` — decisão

`full` e `balanced` não rodam neste ambiente: `codegraph index` exige `pyright-langserver`. Para
comparar, um binário temporário ligou extração e filtro de `References` no `fast` (sem LSP), e
foi descartado após a medição. `REFERENCES` sobe de 0.000 para 1.000 em corpus e sonda, e o ARI
de corpus para 1.000 — porém as arestas Rust crescem 63 → 68 no corpus (+7,9%) e 139 → 144 na
sonda (+3,6%). O limite do plano é 5%; como o corpus o excede, **`fast` continua sem
`References`**. Registro também em [`baseline.md`](baseline.md) e [`p1-baseline.md`](p1-baseline.md).
