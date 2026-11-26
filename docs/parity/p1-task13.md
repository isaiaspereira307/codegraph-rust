# P1 Task 13 — `import_context`, fallbacks de risco e `extra` dos nós

## 1. `edges.extra.import_context` e ciclos de import

Os extratores passam a gravar `import_context` como o `parser.py` do CRG
(`_import_context`, `_python_names_parent_package`, `_js_module_call_is_deferred`):

| linguagem | `type_only` | `deferred` | `parent_package` |
|---|---|---|---|
| Python | import em ramo morto (`if TYPE_CHECKING:`, `if False:`; um `TYPE_CHECKING` solto conta mesmo sem import de `typing`); vence o `deferred` | import dentro de `def` | aresta do pacote em `from M import a, b` quando todo nome é submódulo e o diretório de `M/__init__.py` contém o importador (decidido no indexador, `SymbolCatalog::retain_python_imports`, que conhece os arquivos); não sobrescreve `type_only` |
| JS/TS | `import type X` e `import { type A, type B }` | `import()` sempre; `require` dentro de função | - |
| Ruby | - | `require` dentro de `method`/`singleton_method` | - |

`import_cycles` já ignorava `type_only`/`parent_package` e tirava `deferred` do `load_time`;
agora também deixa fora os arquivos de nós `extra.virtual` (verdadeiro ou 1), como
`get_all_files` do CRG. `GRAPH_SHAPE_VERSION` foi a `crg-shape-16`.

Diferenças conhecidas:

- O CRG mantém `import { type A }` como import real quando o `tsconfig.json` tem
  `verbatimModuleSyntax` ou `importsNotUsedAsValues` `preserve`/`error`. O extrator do Rust não
  lê tsconfig, então marca sempre `type_only`.
- O CRG não emite `autoload` de Ruby (`deferred`); o extrator Ruby do Rust não extrai `autoload`.
- O CRG funde `require('./x')` e `import('./x')` do mesmo arquivo numa aresta com o contexto
  mais forte; o Rust grava uma aresta por chamada. `import_cycles` dá o mesmo resultado (o par
  conta se alguma aresta conta).
- Subpacote de namespace (diretório sem `__init__.py`) conta como submódulo no CRG e não no
  Rust, que só reconhece submódulo que é arquivo.

### Medição neste repositório (2026-11-26)

Clones de `18a431d`, binário debug com o item 1, CRG v2.4.0 da fonte
(`uv run --frozen --extra communities`), `build` completo nos dois lados. Script:
`scripts/parity/cycle_pairs.py` (pares `File → File` de `IMPORTS_FROM` com as regras de `find_import_cycles`).

| | pares `File → File` | depois do filtro de contexto | pares em ciclo | grupos | arestas com contexto |
|---|---|---|---|---|---|
| Rust | 320 | 318 | 15 | 4 | 3 `parent_package`, 2 `deferred` |
| CRG | 320 | 318 | 15 | 4 | 3 `parent_package`, 1 `deferred` |

Os 271 pares da baseline eram de `3840ab1`; o repositório cresceu (sondas de paridade). As três
arestas `parent_package` são as mesmas nos dois lados. O `deferred` a mais no Rust é
`consumer.js → util.ts`, que o CRG funde com o `require` da linha 1 (acima). Gate P1
(`p1_snapshot.sh` + `check_p1.py`): 151 pisos OK, nenhuma métrica caiu.
