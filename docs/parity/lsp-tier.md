# P1 Task 9 — avaliação do tier `balanced` (LSP)

Medição feita em 2026-11-25 com CRG `bbdda4b`, binário debug `09dfde8` e as mesmas fontes do
snapshot P1. Cada alvo foi indexado com `CODEGRAPH_INDEX_TIER=fast|balanced|full` e comparado com
o CRG por `scripts/parity/parity.py run`. O comando de `build` cronometrado é o `codegraph
index . -r` executado dentro do harness; o tempo do CRG não entra na coluna `build`.

## Toolchains disponíveis

Apenas `rust-analyzer` está no PATH. Não há `pyright-langserver`, `typescript-language-server`,
`gopls`, `jdtls` nem `clangd`. O plano proíbe instalar toolchains só para o harness, então as
rodadas `balanced`/`full` de Python, TS/JS, Go e Java param no preflight:

- Python: `Missing pyright-langserver`
- TypeScript/JavaScript: `Missing typescript-language-server`
- Go: `Missing gopls`
- Java: `Missing jdtls`
- `inheritance` (mistura Python/TS/Java): lista `pyright-langserver`,
  `typescript-language-server` e `jdtls`

`CODEGRAPH_ANALYZERS_REQUIRE_TOOLS=0` não contorna: o iniciador do LSP ainda falha com `Missing
required analyzer tool: pyright-langserver`. `CODEGRAPH_ANALYZERS=0` desligaria o próprio LSP e
deixaria de medir o que a tarefa pede; por isso não foi usado.

Rust é a única linguagem com medição `balanced`/`full` real. No corpus Rust os dois tiers
terminam; no probe `rust-imports` (workspace Cargo com `target/`) todos os tiers com LSP falham
em `Project inputs changed during indexing; retry reconciliation` — a validação final vê os
arquivos criados pelo `rust-analyzer` como mudança de entrada. Esse comportamento é registrado
aqui, não corrigido nesta tarefa (escopo: avaliação e decisão de padrão).

## Resultados

`n` = Jaccard de nós; `C` = Jaccard de CALLS; `Cr` = Jaccard de CALLS resolvidas; `I` =
Jaccard de IMPORTS_FROM; `Ir` = Jaccard de IMPORTS_FROM resolvidas; `build` = segundos medidos.
`—` indica falha de preflight/validação; os números entre parênteses em `build` são o tempo até
a falha e não são comparáveis.

| alvo | tier | n | C | Cr | I | Ir | build |
|---|---|---:|---:|---:|---:|---:|---:|
| corpus/python | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.140 |
| corpus/python | balanced | — | — | — | — | — | (0.103) |
| corpus/python | full | — | — | — | — | — | (0.165) |
| corpus/typescript | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.219 |
| corpus/typescript | balanced | — | — | — | — | — | (0.093) |
| corpus/typescript | full | — | — | — | — | — | (0.170) |
| corpus/go | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.201 |
| corpus/go | balanced | — | — | — | — | — | (0.127) |
| corpus/go | full | — | — | — | — | — | (0.149) |
| corpus/rust | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.127¹ |
| corpus/rust | balanced | 0.615 | 0.429 | 0.545 | 0.500 | 1.000 | 0.394¹ |
| corpus/rust | full | 0.500 | 0.429 | 0.545 | 0.500 | 1.000 | 0.427¹ |
| corpus/java | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.224 |
| corpus/java | balanced | — | — | — | — | — | (0.100) |
| corpus/java | full | — | — | — | — | — | (0.107) |
| probe python | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.233 |
| probe python | balanced | — | — | — | — | — | (0.156) |
| probe python | full | — | — | — | — | — | (0.141) |
| probe python-imports | fast | 1.000 | 0.909 | 0.941 | 1.000 | 1.000 | 0.152 |
| probe python-imports | balanced | — | — | — | — | — | (0.124) |
| probe python-imports | full | — | — | — | — | — | (0.178) |
| probe rust-imports | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.159 |
| probe rust-imports | balanced | — | — | — | — | — | (0.380) |
| probe rust-imports | full | — | — | — | — | — | (0.414) |
| probe inheritance | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.147 |
| probe inheritance | balanced | — | — | — | — | — | (0.378) |
| probe inheritance | full | — | — | — | — | — | (0.431) |
| probe typescript-js | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.155 |
| probe typescript-js | balanced | — | — | — | — | — | (0.168) |
| probe typescript-js | full | — | — | — | — | — | (0.152) |
| probe go-package | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.104 |
| probe go-package | balanced | — | — | — | — | — | (0.101) |
| probe go-package | full | — | — | — | — | — | (0.107) |
| probe java-cross-file | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.184 |
| probe java-cross-file | balanced | — | — | — | — | — | (0.087) |
| probe java-cross-file | full | — | — | — | — | — | (0.105) |
| probe java-framework | fast | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.189 |
| probe java-framework | balanced | — | — | — | — | — | (0.104) |
| probe java-framework | full | — | — | — | — | — | (0.118) |

¹ Mediana de três repetições do corpus Rust: fast `0.189/0.125/0.127`, balanced
`0.374/0.394/0.406`, full `0.427/0.398/0.499`. A coluna mostra a mediana. O JSON de cada
rodada está em `/tmp/opencode/p1-t9/strict/`.

Nos alvos com LSP disponível os números de flows e comunidades também caem: corpus Rust fast
entry 1.000 / ARI 1.000, balanced entry 0.667 / ARI 0.451, full entry 0.667 / ARI 0.830.
`IMPORTS_FROM resolvido` é 1.000 em todos os tiers do corpus Rust, mas o Jaccard de
IMPORTS_FROM cai para 0.500 porque o enriquecimento renomeia os nós de destino.

## Onde o LSP fica atrás do CRG

No corpus Rust, `balanced` e `full` trocam nomes canônicos do CRG por nomes do `rust-analyzer`:

- CRG `sample_rust.rs::InMemoryRepo.find_by_id` / `Repository.save` / `S.run`;
- Rust com LSP `sample_rust.rs::impl InMemoryRepo::new`,
  `sample_rust.rs::impl Repository for InMemoryRepo::find_by_id`,
  `sample_dead_guard_impl.rs::impl S::run`.

Essa divergência de `qualified_name` derruba os Jaccards de nós/CALLS/IMPORTS_FROM e os entry
points de flows. Não há caso medido em que o LSP supere o CRG: nas seis linguagens do corpus e
nas oito sondas, `fast` é igual ou melhor em todas as métricas comparáveis. O único número que
melhora com `full` é o ARI do corpus Rust (0.451 no `balanced` → 0.830 no `full`), ainda abaixo
do 1.000 do `fast`.

## Decisão

**Manter `fast` como padrão.** A regra do plano exige que `balanced` ganhe ≥ 0.05 em CALLS em
pelo menos três linguagens **e** custe ≤ 20% a mais em `build`. Não houve nenhum ganho
comparável (as quatro linguagens sem LSP não chegaram a medir; a única que mediu piorou de
1.000 para 0.429) e o custo no corpus Rust subiu de 0.127 s para 0.394 s — 3,1×, ou 210% a
mais, contra o limite de 20%. Não há mudança de padrão, docs de configuração ou testes a
atualizar.

## Reprodução

```bash
# Wrapper que cronometra só o `codegraph index` (o harness também chama o CRG).
CODEGRAPH_INDEX_TIER=balanced CG_TIMING_LOG=/tmp/opencode/p1-t9/corpus-rust-balanced.time \
  python3 scripts/parity/parity.py run /tmp/opencode/p1-t9/sources/corpus/rust \
    --work /tmp/opencode/p1-t9/recheck \
    --crg-repo /tmp/opencode/parity-p1-t8/work/crg-repo \
    --codegraph /tmp/opencode/p1-t9/bin/codegraph \
    --json /tmp/opencode/p1-t9/recheck.json --md /tmp/opencode/p1-t9/recheck.md
```

As fontes por linguagem em `/tmp/opencode/p1-t9/sources/corpus/` são uma cópia dos fixtures
nomeados em `scripts/parity/corpus.json`; o log completo por alvo/tier está em
`/tmp/opencode/p1-t9/strict/`.
