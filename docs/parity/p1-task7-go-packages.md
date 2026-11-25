# P1 Task 7 — imports de pacote e arquivos de teste em Go

Port dos resolvers de `go.mod` do CRG (`_go_module_chain`, `_resolve_go_package_dir`) e da
ligação seletiva de chamadas por evidência de import. Medido em 2026-11-25 com CRG `bbdda4b`,
binário debug e tier `fast` padrão. Limites congelados em
[`p1-baseline.md`](p1-baseline.md).

## Probe e vermelho

`scripts/parity/probes/go-package/` tem um módulo `example.com/probe` com:

- `pkg/helper/helper.go` — pacote do próprio módulo;
- `third_party/dep/` + `replace example.com/dep => ./third_party/dep` — módulo substituído;
- `vendor/example.com/vendored/` — pacote vendored (ignorado via `.gitignore`, como no CRG);
- `app/main_test.go` — arquivo `_test.go` com `TestRun`.

Vermelho authoritative, com o binário do `a137eca` (antes desta task):

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/go-package \
  --work /tmp/opencode/p1-t7-red-authoritative \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph /tmp/opencode/p1-t7-red-target/debug/codegraph \
  --json /tmp/opencode/p1-t7-red-authoritative.json \
  --md /tmp/opencode/p1-t7-red-authoritative.md
```

| métrica | vermelho | verde | meta |
|---|---|---|---|
| nós `go` | 1.000 | 1.000 | ≥ 0.95 |
| CALLS | 0.333 (resolvido 0.000) | 1.000 (resolvido 1.000) | ≥ 0.95 |
| CONTAINS | 1.000 | 1.000 | ≥ 0.95 |
| IMPORTS_FROM | 0.333 | 1.000 | ≥ 0.95 |
| TESTED_BY | 1.000 | 1.000 | ≥ 0.95 |
| flows (entry/membros) | 0.000 / — | 1.000 / 1.000 | — |
| ARI de comunidades | 1.000 (detector `file`) | 1.000 (detector `leiden`) | — |

O vermelho mostrava `IMPORTS_FROM` com o texto do módulo (`example.com/probe/pkg/helper`,
`example.com/dep`) em vez do diretório do pacote, e `helper.Help()`/`dep.Dep()` ligados ao nome
solto `Help`/`Dep`. `is_test=1` e `TESTED_BY` já casavam antes da task.

## O que mudou

Commit `8993316`:

1. `ModuleIndex` descobre `go.mod` nos ancestrais dos arquivos indexados, lê `module` e
   `replace` (linha única ou bloco), e resolve `example.com/probe/pkg/helper` e
   `example.com/dep` para diretórios com fonte `.go` indexada. O `set_go_root` do `SymbolCatalog`
   converte os nomes relativos que o indexer usa para o root absoluto do projeto, que é a
   grafia que o CRG grava e o harness compara. `vendor/` entra na resolução quando há fonte
   indexada no diretório; no probe ele está ignorado e o import permanece como texto.
2. `IMPORTS_FROM` de Go vira aresta não resolvida com o diretório como `target_qualified` e
   `metadata.import_scope = "package"`, sem criar nó `Directory`/`Module` — o CRG não emite nó
   para o diretório.
3. Chamadas seletoras conservam o qualificador em `metadata.go_package_qualifier` sem alterar
   o alvo solto extraído (`helper.Help()` continua com alvo `Help`). A resolução liga o alvo
   somente quando o pacote importado pelo arquivo contém exatamente um símbolo com esse nome.
   `vendored.Value()` e `fmt.Printf()` continuam soltos quando não há evidência única.

Verde:

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/go-package \
  --work /tmp/opencode/p1-t7-green4 \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t7-green4.json --md /tmp/opencode/p1-t7-green4.md
```

## Ramo ambíguo do plano (R8)

O plano pedia "portar o ramo de nome solto ambíguo do CRG que liga ao primeiro símbolo do
arquivo". No commit congelado do CRG esse comportamento não existe: `b79e3a7` removeu o fallback
`package_files[0]`, e a resolução atual exige evidência de same-file/import e candidato único;
com múltiplos candidatos o endpoint permanece solto e é marcado como ambíguo
(`tests/test_graph.py:1702-1712`, `tests/test_postprocessing.py:390-456`). Portar "primeiro
símbolo" criaria ligações que o CRG não cria e poderia derrubar os pisos de risco. A decisão está
registrada como R8 no ledger; o probe cobre o caso de candidato único, que é o que o CRG liga.

## Corpus e detector

Corpus Go segue em 1.000 em nós, CALLS, CONTAINS, IMPORTS_FROM, flows e ARI. `p1_snapshot.sh`
passou a incluir o probe `go-package`, e `check_p1.py` ganhou cinco pisos (nós/CALLS/CONTAINS/
IMPORTS_FROM/TESTED_BY). Snapshot completo:

```bash
bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-t7
python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t7
# P1 baseline OK: 91 floors, no metric dropped
```

`cargo check --workspace` limpo; `cargo test -p codegraph-parser --lib` 114/114 e
`cargo test -p codegraph-mcp --lib` 104/104. A falha pré-existente
`python_submodule_imports_and_import_bound_calls_follow_crg` documentada na Task 5 permanece
fora do escopo. O `update` da rodada do snapshot levou 70,10 s no Rust debug contra 16,19 s no
CRG; a medição definitiva de performance fica na Task 11.
