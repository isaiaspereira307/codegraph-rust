# P1 Task 4 — reexportação de pacote e `sys.modules`

Port da resolução de origem de reexport (`../code-review-graph/code_review_graph/parser.py:22720`)
e dos shims `sys.modules[__name__] = importlib.import_module("...")`. Medido em 2026-11-25 com CRG
`bbdda4b`, binário debug e tier `fast` padrão. Limites congelados em
[`p1-baseline.md`](p1-baseline.md).

## O que mudou

- O probe `scripts/parity/probes/python-imports` ganhou `engine.py`/`billing_lib.py`
  (`billing_lib` reexporta `run_model` de `engine`), `billing.py` (shim para `billing_lib`),
  `legacy_billing.py` (shim de um shim) e chamadas em `plain.py`. Os módulos são achatados de
  propósito: diretórios extras só gerariam nós `Directory` no Rust e derrubariam CONTAINS.
- `ModuleIndex` lê, uma vez por arquivo Python indexado, as declarações top-level de
  `sys.modules`/`from X import Y`; shims são seguidos até o módulo real (com guarda de ciclo) e
  `SymbolCatalog::py_import_target` troca o arquivo que só reexporta pelo arquivo que define o
  símbolo. Nomes não definidos no módulo real (`billing.refund`) continuam como texto.

## Medição

Comando (sonda, sem LSP):

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/python-imports \
  --work /tmp/opencode/p1-t4-green2 \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t4-green2.json --md /tmp/opencode/p1-t4-green2.md
```

| métrica | antes (com as novas fixtures) | depois | piso/meta |
|---|---|---|---|
| nós python | 1.000 | 1.000 | 1.000 |
| CALLS | 0.720 | 0.909 | ≥ 0.90 |
| CALLS resolvido | 0.824 | 0.941 | ≥ 0.90 |
| IMPORTS_FROM | 0.750 | 1.000 | 1.000 |
| CONTAINS | 0.920 | 0.920 | 0.909 |

Snapshot completo: `bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-t4` seguido de
`python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t4` → **82 pisos, nenhum métrica
caiu**.

As duas chamadas que ainda divergem são as conhecidas do baseline: `plain.py::caller -> h`
(ambiguidade de nome solto) e `q/amb.py::use -> q/lib.py::Svc.go` (reexport ambíguo mantido como
texto, conforme o plano).
