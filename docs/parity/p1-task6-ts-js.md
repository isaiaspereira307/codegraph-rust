# P1 Task 6 — paridade TypeScript/JavaScript

Port das sobras do extrator do CRG (`_js_wrapped_function`, JSX como `CALLS`, `_extract_js_module_call`,
`_extract_js_member_functions`, guarda morta estática) e dos resolvers de workspace/npm.
Medido em 2026-11-25 com CRG `bbdda4b`, binário debug e tier `fast` padrão. Limites congelados em
[`p1-baseline.md`](p1-baseline.md).

## Probe e vermelho

O probe `scripts/parity/probes/typescript-js/` cobre um caso por item:

- `src/app.tsx`: JSX (`<Widget />`, `div` intrínseco), `memo`/`forwardRef`, chamada em callback,
  guarda `if (false)`, receptor de objeto solto (`users.get()`).
- `src/consumer.js`: `require('./util')`, `import('./util')`, `require('@ws/shared')`,
  `exports.run = function`, `Thing.prototype.go = function`, guarda `if (false)`.
- `package.json` + `packages/shared/package.json`: workspace npm que dá nome a `@ws/shared`.
  Ambos os `package.json` estão no `.gitignore` do probe: o CRG precisa deles para resolver o
  alias, mas não deve indexar nós `json` apenas de um lado.

Vermelho authoritative, com o probe final e o binário do `e35dec2` (antes desta task):

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/typescript-js \
  --work /tmp/opencode/p1-t6-red-final \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph /tmp/opencode/p1-t6-red-target/debug/codegraph \
  --json /tmp/opencode/p1-t6-red-final.json --md /tmp/opencode/p1-t6-red-final.md
```

| métrica | vermelho | verde | meta |
|---|---|---|---|
| nós `javascript` | 0.714 | 1.000 | 1.000 |
| nós `tsx` | 0.333 | 1.000 | 1.000 |
| nós `typescript` | 1.000 | 1.000 | 1.000 |
| CALLS | 0.200 (resolvido 0.067) | 1.000 (resolvido 1.000) | ≥ 0.95 |
| CONTAINS | 0.471 | 1.000 | ≥ 0.95 |
| IMPORTS_FROM | 0.500 (resolvido 0.333) | 1.000 (resolvido 1.000) | ≥ 0.95 |
| INHERITS / TESTED_BY | ausentes nos dois lados | ausentes | ≥ 0.95 |
| flows (entry/membros) | 0.000 / — | 1.000 / 1.000 | 1.000 |
| ARI de comunidades | 0.000 | 1.000 | — |

O vermelho mostrava: só 2 nós em `.tsx` (a gramática TS não entendia JSX), nenhum nó
`exports.run`/`Thing.prototype.go`, chamadas JSX atribuídas ao arquivo, `@ws/shared` sem
resolução, `require` sem `IMPORTS_FROM`, chamadas sob `if (false)` e quatro nós de diretório
(`unknown`) apenas no Rust.

## O que mudou

Lotes comitados, em ordem:

1. `5f1d5a2` — Calls sob guarda estaticamente falsa deixam de ser emitidos; depois `f38a248`
   ajustou a regra ao CRG (só o consequente de `if (false)`/`if (0)` é morto; `else` permanece
   vivo, incluindo declarações aninhadas).
2. `00a6d70` — `require(...)` e `import(...)` com literal estático emitem `IMPORTS_FROM` no arquivo;
   `ae0a52f` preserva a chamada solta a `require` que o CRG também registra.
3. `b395ad8` — atribuições de função a membro (`exports.f`, `module.exports.f`,
   `Foo.prototype.bar`) viram nós `Function` com `CONTAINS`.
4. `e63f1dc` — `.tsx` usa `LANGUAGE_TSX` (`Language::Tsx` no registry, watch, LSP e visitantes);
   tags JSX com nome maiúsculo viram `CALLS`, tags DOM intrínsecas não; `memo`, `forwardRef` e os
   demais wrappers conhecidos viram o nó da função embrulhada, com as chamadas aos wrappers no
   escopo do arquivo.
5. `4b3a52f` — `ModuleIndex` lê `package.json` dos ancestrais dos arquivos indexados, descobre
   workspaces (`packages/*`), aliases `npm:` e `exports`/`main` de pacotes locais. Isso resolve
   `@ws/shared` para `packages/shared/index.js` e permite que `shared.shared()` ligue ao nó do
   pacote. Nomes de pacotes escopados (`@scope/name/subpath`) são tratados como um só nome.
6. `905d4fb` — nós sintéticos de diretório e suas arestas `CONTAINS` saíram do caminho de
   indexação (agora exigem `with_directory_nodes(true)`). O corpus plano do P1 não os via; o probe
   com subdiretórios expôs 4 nós e 3 arestas `unknown` que o CRG nunca emite. O cache AST inclui o
   sinalizador na chave para não reaproveitar resultados com diretórios.

Verde (mesmo comando, binário atual):

```bash
python3 scripts/parity/parity.py run scripts/parity/probes/typescript-js \
  --work /tmp/opencode/p1-t6-green2 \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t6-green2.json --md /tmp/opencode/p1-t6-green2.md
```

Todas as arestas CALLS/CONTAINS/IMPORTS_FROM casam, incluindo linha (`IMPORTS_FROM` tem
concordância de linha 0.750 porque o CRG consolida as duas cargas de `@ws/shared` na primeira;
os alvos e a resolução são idênticos).

## Corpus e detector

`python3 scripts/parity/parity.py corpus ...` com o binário atual: corpus `typescript` segue em
1.000 em nós, CALLS, CONTAINS, IMPORTS_FROM, INHERITS, TESTED_BY, flows e ARI; os demais
idiomas do corpus também permaneceram em 1.000 nas suas métricas.

`p1_snapshot.sh` passou a incluir o probe `typescript-js`, e `check_p1.py` ganhou quatro pisos
(nós/CALLS/CONTAINS/IMPORTS_FROM do probe). Snapshot completo:

```bash
bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-t6
python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t6
# P1 baseline OK: 86 floors, no metric dropped
```

`cargo check --workspace` limpo; `cargo test -p codegraph-parser --lib` 113/113 e
`cargo test -p codegraph-mcp --lib` 102/102. A falha pré-existente
`python_submodule_imports_and_import_bound_calls_follow_crg` documentada na Task 5 permanece
fora do escopo.

## Divergência conhecida: tipo do receptor

O probe mantém `users.get()` sem anotação de tipo; os dois lados emitem o alvo solto `get`.
Quando o tipo está presente e é ambíguo, o CRG usa a evidência `receiver_type` e o Rust não:

```ts
class Repo { get() { return 1; } }
class Other { get() { return 2; } }
class Service {
  repo: Repo;
  run() { this.repo.get(); }
}
```

Em uma cópia descartável do probe com esse arquivo, o CRG emite
`typed.ts::Service.run -> typed.ts::Repo.get` e o Rust emite `-> get`; CALLS cai para 0.895
(resolvido 0.923). Com apenas uma classe `get` no arquivo, com tipo importado de outro arquivo ou
com campo tipado em classe separada, os dois lados resolvem igual. Essa é a parte que depende de
tipo do receptor e fica fora dos pisos da Task 6.

`REFERENCES` continua desligado no tier `fast` (decisão da Task 3): o probe tem uma aresta só no
CRG (`renderAll -> helper`), sem piso P1. O `update` da rodada do snapshot levou 126,9 s no Rust
debug contra 21,25 s no CRG; a medição definitiva de performance fica na Task 11.
