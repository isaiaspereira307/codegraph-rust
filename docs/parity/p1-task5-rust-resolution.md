# P1 Task 5 — resolução Rust entre arquivos e custo por aresta

Port das sobras de resolução do CRG (`_resolve_module_to_file`, `_resolve_rust_scoped_call`,
`_resolve_rust_type_target`) e redução da alocação feita por aresta em `SymbolCatalog::exact`.
Medido em 2026-11-25 com CRG `bbdda4b`, binário debug e tier `fast` padrão. Limites congelados em
[`p1-baseline.md`](p1-baseline.md).

## O que mudou

- `ModuleIndex::rust` agora limita `super::` ao raiz do crate, como o CRG: um `super` extra não
  devolve `None`, e `super::config::load()` num arquivo da raiz continua ligando a
  `config.rs::load`.
- O probe `rust-imports` ganhou `repo.rs` (receptor tipado `let repo = Repo::new(); repo.save()`),
  `store/alt.rs` (`#[path]`), `pub use config::Tr as Exported` (reexport de trait) e
  `pub use super::config::Tr as Ship` em `store/mod.rs`. O receptor tipado permanece sem ligação
  nos dois lados (`save` solto), como divergência conhecida.
- `SymbolCatalog::reachable` devolve `Cow<[NodeId]>`: nomes qualificados (o caso comum) são
  emprestados; só uma busca com nome solto copia a lista filtrada. `exact_where` e `fuzzy`
  adaptaram-se sem mudar a precedência de resolução.
- Teste de integração `manifest_only_edit_re_resolves_rust_sources`: editar só `Cargo.toml`
  invalida o fingerprint de entradas e re-resolve as fontes (o alvo `other::thing::remote_only`
  passa a ligar a `other/src/thing.rs::remote_only`).

## Medição

Comando da sonda:

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/rust-imports \
  --work /tmp/opencode/p1-t5-green \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t5-green.json --md /tmp/opencode/p1-t5-green.md
```

| métrica | antes (com as novas fixtures) | depois | piso/meta |
|---|---|---|---|
| nós rust | 1.000 | 1.000 | 1.000 |
| CALLS (probe) | 0.867 | 1.000 | 1.000 |
| IMPORTS_FROM (probe) | 1.000 | 1.000 | 1.000 |
| IMPLEMENTS (probe) | 1.000 | 1.000 | 1.000 |
| CONTAINS (probe) | 0.821 | 0.821 | 0.737 |

O corpus Rust deste repositório permaneceu em 1.000 em nós, CALLS, IMPORTS_FROM, IMPLEMENTS,
CONTAINS e TESTED_BY. Snapshot completo:
`bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-t5` seguido de
`python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t5` → **82 pisos, nenhuma métrica
caiu**.

Custo de `codegraph build . --skip-postprocess` (release, tier fast, neste repositório): 10 pares
A/B alternados com binários do HEAD e da mudança (`/tmp/opencode/t5-ab-skip-times.log`) deram
mediana **16,46 s → 15,51 s** (sem regressão mensurável; a dispersão vai de 12,4 s a 19,7 s em
ambos os lados). Com pós-processo completo, a mesma comparação teve medianas 12,70 s → 12,77 s,
dentro do ruído.

## Validação e falha pré-existente

`cargo test -p codegraph-mcp --lib`: 100/100; `cargo test -p codegraph-parser`: verde;
`cargo test -p codegraph-mcp --test reconciliation_test`: 20/21. O teste
`python_submodule_imports_and_import_bound_calls_follow_crg` já falhava em `590d57d` (antes do P1),
`8ced807`, `70c8ac1` e `5cedae5`: `Thing.go()` fica como `go` e aparece ao lado de `x.go()`,
enquanto o teste espera `pkg/b.py::Thing.go`. Confirmado em quatro worktrees isolados sem as mudanças
desta task; não é regressão do P1 e fica registrado para o fechamento.

## Divergências conhecidas

- `impl Trait for T` com `T` importado de outro arquivo: o CRG emite uma aresta cuja origem é
  `arquivo.rs::T` mesmo sem nó com esse nome; o extrator Rust exige um nó real e não reproduz
  essa aresta órfã. Não há piso congelado para ela.
- `use m::*` continua sem alias de símbolo nos dois lados — coberto pelos testes de
  `flatten_use` em `rust.rs`.
