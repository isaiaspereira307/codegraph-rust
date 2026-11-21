# Fase 2 / TypeScript-JS: paridade do extrator com o CRG — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** levar o corpus `typescript` do harness de paridade a Jaccard 1.0 em nós, CALLS,
CONTAINS, IMPORTS_FROM, INHERITS e TESTED_BY, reescrevendo o extrator TS/JS para gerar o
que o CRG gera (métodos, interfaces/tipos, nomes de chamada, nós de teste do vitest/jest,
herança).

**Architecture:** O `TypeScriptCollector` (`crates/codegraph-parser/src/languages/javascript.rs`)
passa a percorrer a árvore com uma pilha de escopo (classe atual, função nomeada atual) em vez
do `current_function_id` que nunca é limpo. Decisões que dependem do arquivo inteiro continuam
no `crg_shape`: ele já qualifica nomes e gera `CONTAINS`; ganha duas marcas novas vindas do
extrator (nó de test runner, chamada no escopo do módulo). A resolução de nomes soltos que o
plano 1 fez (`resolution.rs`: mesmo arquivo ou arquivo importado, candidato único) já liga
`getUser`/`findById` como o CRG, desde que o extrator grave o nome final da chamada.

**Tech Stack:** Rust, `tree-sitter-typescript` (`LANGUAGE_TYPESCRIPT`), `tree-sitter-javascript`
já registrados; harness Python de `scripts/parity/`.

**Spec:** `PLANO.md`, Fase 2, item "Próximos planos por linguagem" 1. Referência:
`../code-review-graph` no commit `bbdda4b`, `code_review_graph/parser.py`:
`_CLASS_TYPES`/`_FUNCTION_TYPES` l.1450/1549 (TS), `_TEST_RUNNER_NAMES` l.1881,
`_TS_HERITAGE_CLAUSES` l.1538, `_get_test_description` l.10633 (`_MAX_TEST_DESCRIPTION_LEN = 200`),
`_extract_js_var_functions` l.14935, `_extract_js_field_function` l.15097, `_extract_calls`
l.17274, `_get_bases` l.23802 (ramo TS l.23858), `_get_call_name` l.24579,
`_get_base_call_name`/`_TEST_MODIFIER_SUFFIXES` l.24868.

## Global Constraints

- Mesmas do plano 1 (`2026-11-20-fase2-paridade-parser.md`): CRG da fonte via `uv run --frozen`,
  IDs SHA e `schema_v15.sql` intocados, arquivos novos com duas linhas `ABOUTME:`.
- JavaScript usa o mesmo coletor (`Language::JavaScript`); toda regra vale para os dois.
- Fora do escopo: `REFERENCES` (anotações de tipo; também falta em Python, vira plano comum),
  `line` das arestas, guardas mortas (`if (false)`), TSX/JSX, aliases npm e workspaces.
- `GRAPH_SHAPE_VERSION` (`crates/codegraph-mcp/src/indexer.rs:44`) passa de `"crg-shape-1"` para
  `"crg-shape-2"`: índices antigos são reconstruídos.

## Forma do CRG para TS (medida no corpus `typescript`)

Nós (`kind name parent_name`):

```
Class UserData            (interface_declaration vira Class)
Class UserRepository
Function findById UserRepository
Function save UserRepository
Class UserService
Function getUser UserService
Function createUser UserService
Function handleGetUser
Test describe:UserService@L3          (sample_vitest.test.ts, linhas 3–18)
Test it:should create a user@L4       (4–7)
Test it:should find a user by id@L9   (9–13)
Test test:alternative test syntax@L15 (15–17)
```

- Classes: `class_declaration`, `abstract_class_declaration`, `class` (com nome),
  `interface_declaration`, `type_alias_declaration`, `enum_declaration`; todas `kind=Class`.
- Funções: `function_declaration`, `generator_function_declaration`, `method_definition`,
  `abstract_method_signature`; `arrow_function`/`function_expression` só quando são o valor de um
  `variable_declarator` (nome = variável) ou de um `public_field_definition` (nome = campo).
  Arrow anônima não vira nó; as chamadas dela ficam com o escopo nomeado que a envolve.
- Nome da chamada (`_get_call_name`): callee `identifier` → o texto; callee `member_expression`
  → a propriedade mais à direita (`res.status(404).json` → `json`, `this.users.get` → `get`);
  qualquer outro callee (`super(...)`, `import(...)`, `f()()`, `a[0]()`, `new X()`) → sem aresta.
- Chamada fora de qualquer função → `CALLS <arquivo> → nome`.
- Em arquivo de teste (`is_test_file`), chamada a `describe|it|test|beforeEach|afterEach|
  beforeAll|afterAll|suite` (ou `describe.only`, `it.skip`… com sufixo em
  `only|skip|each|todo|concurrent|failing`) não gera CALLS: gera nó `Test` chamado
  `<runner>:<descrição>@L<linha>` (ou `<runner>@L<linha>` sem descrição), linhas da chamada,
  `CONTAINS` do runner/função que o envolve (senão do arquivo), e as chamadas do callback
  saem desse nó. Descrição: primeiro argumento `string`/`template_string`, sem `'"\``
  nas pontas, espaços colapsados, até 200 caracteres.
- `INHERITS Classe → Base` como texto, nunca resolvido (mesmo com `Base` no mesmo arquivo), para
  cada nome em `extends_clause`/`implements_clause` (sob `class_heritage`) e
  `extends_type_clause` (interface): filhos diretos `identifier`/`type_identifier`/
  `nested_identifier`; de `generic_type`, a cabeça (`type_identifier`/`nested_type_identifier`).

Arestas esperadas depois do plano (CALLS, como o harness as mostra):

```
alias_importer.ts::formatUser -> cn
sample_typescript.ts::UserRepository.findById -> get
sample_typescript.ts::UserRepository.save -> set
sample_typescript.ts::UserService.getUser -> sample_typescript.ts::UserRepository.findById
sample_typescript.ts::UserService.createUser -> now
sample_typescript.ts::UserService.createUser -> sample_typescript.ts::UserRepository.save
sample_typescript.ts::handleGetUser -> sample_typescript.ts::UserService.getUser
sample_typescript.ts::handleGetUser -> Number
sample_typescript.ts::handleGetUser -> json
sample_typescript.ts::handleGetUser -> status
sample_vitest.test.ts::it:should find a user by id@L9 -> sample_typescript.ts::UserRepository.findById
```

## Review Focus

1. Arrow com parâmetro sem parênteses (`items.map(x => f(x))`): hoje vira função chamada `x`;
   não pode virar nó, e `f` sai da função que envolve. Teste na Task 1.
2. Chamada no topo do módulo depois de uma função (`function a(){}; main();`): hoje é atribuída
   a `a`; precisa sair do arquivo. Teste na Task 2.
3. `describe`/`it` fora de arquivo de teste (`src/a.ts`): é chamada comum (`CALLS → it`), sem
   nó `Test`. Teste na Task 3.
4. Runners aninhados e modificadores (`describe.only` contendo `it.skip`): `CONTAINS describe → it`,
   nome com o runner base. Teste na Task 3.
5. Herança com classe no mesmo arquivo continua texto depois do indexer (não liga ao nó
   `Base`). Teste na Task 4.

Risco aceito, registrar no PLANO: chamada com receptor (`this.users.get`) é gravada só como
`get` e pode ligar a um `get` único no mesmo arquivo/importado; o CRG usa o tipo do receptor
para evitar isso. `this.x()` com dois métodos `x` no arquivo fica sem resolver (o CRG qualifica
pela classe e sobe a herança).

---

### Task 1: Declarações e escopo do coletor TS

**Files:**
- Modify: `crates/codegraph-parser/src/languages/javascript.rs` (coletor inteiro; apagar o
  `extract_js_ts_nodes` morto se nada o usar — `grep -rn extract_js_ts_nodes crates`)
- Test: módulo `#[cfg(test)] mod tests` no fim de `javascript.rs`

**Interfaces:**
- Produces: `TypeScriptExtractor::extract_with_edges(tree: &Tree, content: &str, file_path: &str, language: Language, test_file: bool) -> ExtractionResult`.
  O impl de `LanguageExtractor` passa `test_file = false`.
- Produces: `extract_unshaped` em `languages/mod.rs` recebe `rel_path: &str` e passa
  `test_detection::is_test_file(rel_path)` para TS e JS (único chamador: `extract_for_language_at`).
- Nós de classe: `NodeType::Interface` para `interface_declaration`, `NodeType::Class` para os
  demais tipos de classe da lista acima (`type_alias_declaration` e `enum_declaration` também
  `Class`, porque `crg_node_kind` mapeia `Enum`/`Type` para `Type`).

- [ ] **Step 1: Write the failing tests**

Helper de teste `extract_ts(source, path) -> ExtractionResult` via
`crate::languages::extract_for_language_at(&Language::TypeScript, …, path, path, Default::default())`
e `qualified_names` como em `crg_shape.rs`. Testes:

```rust
#[test]
fn class_methods_interfaces_and_types_are_nodes() {
    let r = extract_ts(
        "interface D { id: number }\ntype T = string;\nenum E { A }\n\
         abstract class Base { abstract run(): void; }\n\
         class S extends Base {\n  get(id: number) { return id; }\n  handler = () => go();\n}\n\
         export const f = (a: number) => a;\nfunction g() {}\n",
        "m.ts",
    );
    let qns = qualified_names(&r);
    for q in ["m.ts::D", "m.ts::T", "m.ts::E", "m.ts::Base", "m.ts::Base.run",
              "m.ts::S", "m.ts::S.get", "m.ts::S.handler", "m.ts::f", "m.ts::g"] {
        assert!(qns.contains(q), "{q} missing from {qns:?}");
    }
}

#[test]
fn anonymous_arrows_are_not_nodes() {
    let r = extract_ts("function outer() {\n  [1].map(x => inner(x));\n}\n", "m.ts");
    let qns = qualified_names(&r);
    assert!(!qns.iter().any(|q| q.ends_with("::x")), "{qns:?}");
    assert_eq!(calls_from(&r, "m.ts::outer"), vec!["inner", "map"]);
}
```

`calls_from(&r, qualified) -> Vec<String>`: alvos (`edge.to`) das arestas `Calls` cujo `from`
é o nó com esse `qualified_name`, ordenados.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codegraph-parser languages::javascript`
Expected: FAIL (`m.ts::S.get missing`, nó `x` presente).

- [ ] **Step 3: Implement the collector**

Pilha de escopo explícita: entrar num nó de classe empilha a classe; entrar numa função nomeada
empilha seu `NodeId`; desempilhar ao sair da subárvore. Chamadas usam o topo da pilha de funções.
Nome de `method_definition`/`abstract_method_signature` pelo campo `name`; de
`variable_declarator` pelo campo `name` quando o campo `value` é `arrow_function` ou
`function_expression` (a função aninhada não gera um segundo nó); de `public_field_definition`
pelo `property_identifier` quando o valor é função. Classe pelo campo `name`; `class` sem nome é
ignorada. Manter `with_content`, `with_complexity` e `span` como hoje.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codegraph-parser`
Expected: PASS (inclui `runner_names_are_tests_only_in_test_files` de `crg_shape.rs`).

- [ ] **Step 5: Commit**

```bash
git add crates/codegraph-parser/src/languages/javascript.rs crates/codegraph-parser/src/languages/mod.rs
git commit -m "feat(parser): TS methods, interfaces and named arrows like CRG"
```

### Task 2: Nomes de chamada e chamadas no escopo do módulo

**Files:**
- Modify: `crates/codegraph-parser/src/languages/javascript.rs`
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs`

**Interfaces:**
- Consumes: Task 1.
- Produces: `pub const FILE_SCOPE_ATTR: &str = "crg_file_scope";` em `crg_shape.rs`. Uma aresta
  com `metadata[FILE_SCOPE_ATTR] == "true"` sai do nó `File`: `shape_for_crg` troca `edge.from`
  pelo id do `File` e remove a chave. O `from` que o extrator grava nessas arestas é
  `NodeId::nil()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn call_names_are_the_rightmost_member() {
    let r = extract_ts(
        "function h(res) {\n  res.status(404).json({});\n  this.users.get(1);\n  Date.now();\n\
         Number(1);\n  new Map();\n  super.x();\n  (0, f)();\n}\n",
        "m.ts",
    );
    assert_eq!(calls_from(&r, "m.ts::h"), vec!["Number", "get", "json", "now", "status", "x"]);
}

#[test]
fn module_scope_calls_leave_the_file() {
    let r = extract_ts("function a() {}\nmain();\n", "m.ts");
    assert!(calls_from(&r, "m.ts::a").is_empty());
    assert_eq!(calls_from(&r, "m.ts"), vec!["main"]);
    assert!(r.edges.iter().all(|e| !e.metadata.contains_key(FILE_SCOPE_ATTR)));
}
```

(`super.x()` é `member_expression` com objeto `super` → `x`, como o CRG; `super(...)` puro não
gera aresta.) `calls_from(&r, "m.ts")` busca o nó `File`, cujo `qualified_name` é `m.ts`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codegraph-parser languages::javascript`
Expected: FAIL (alvos `res.status(404).json`, `main` atribuído a `a`).

- [ ] **Step 3: Implement call naming and file-scope marking**

`fn call_name(&self, call: &Node) -> Option<String>` segue a regra da seção "Forma do CRG";
`new_expression` não é chamada. Sem função no topo da pilha, a aresta usa `NodeId::nil()` e
`FILE_SCOPE_ATTR`. Em `shape_for_crg`, reapontar essas arestas junto do laço que já reaponta
arestas de import.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codegraph-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codegraph-parser/src/languages/javascript.rs crates/codegraph-parser/src/languages/crg_shape.rs
git commit -m "feat(parser): CRG call names and module-scope calls for TS"
```

### Task 3: Nós de teste de runners (vitest/jest/mocha)

**Files:**
- Modify: `crates/codegraph-parser/src/languages/javascript.rs`
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs`
- Modify: `crates/codegraph-parser/src/languages/test_detection.rs` (expor
  `pub fn is_test_runner_name(name: &str) -> bool` sobre `TEST_RUNNER_NAMES`, se ainda não houver)

**Interfaces:**
- Consumes: `test_file` da Task 1, `call_name` da Task 2.
- Produces: `pub const TEST_RUNNER_ATTR: &str = "crg_test_runner";` em `crg_shape.rs`. O extrator
  grava o runner como `NodeType::Function` com esse atributo `"true"`, nome sintético e linhas da
  chamada. `shape_for_crg` qualifica como os demais membros (`<arquivo>::[Classe.]nome`), mas:
  o `CONTAINS` vem do nó `Function` ou runner mais interno que o contém por extensão (senão da
  regra atual: classe ou arquivo); vira `Test` com `is_test=true` e `PRE_SHAPE_KIND_ATTR` como
  as funções de teste já viram.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn runner_calls_in_test_files_become_tests() {
    let src = "describe.only('S', () => {\n  it.skip(`finds  one`, () => { svc.findById(1); });\n\
               beforeEach(() => {});\n});\n";
    let r = extract_ts(src, "a.test.ts");
    let qns = qualified_names(&r);
    for q in ["a.test.ts::describe:S@L1", "a.test.ts::it:finds one@L2", "a.test.ts::beforeEach@L3"] {
        assert!(qns.contains(q), "{q} missing from {qns:?}");
    }
    let edges = contains_edges(&r);
    assert!(edges.contains(&("a.test.ts".into(), "a.test.ts::describe:S@L1".into())));
    assert!(edges.contains(&("a.test.ts::describe:S@L1".into(), "a.test.ts::it:finds one@L2".into())));
    assert_eq!(calls_from(&r, "a.test.ts::it:finds one@L2"), vec!["findById"]);
    assert!(calls_from(&r, "a.test.ts").is_empty());
    let it = node(&r, "a.test.ts::it:finds one@L2");
    assert_eq!(it.node_type, Some(NodeType::Other("Test".into())));
    assert_eq!(id_node_type(it), "Function");
    assert_eq!((it.location.line, it.location.end_line), (2, Some(2)));
}

#[test]
fn runner_calls_outside_test_files_are_plain_calls() {
    let r = extract_ts("describe('S', () => { go(); });\n", "src/a.ts");
    assert!(!qualified_names(&r).iter().any(|q| q.contains('@')));
    assert_eq!(calls_from(&r, "src/a.ts"), vec!["describe", "go"]);
}
```

Mais um teste da descrição: string com 250 caracteres vira nome com exatamente 200 caracteres de
descrição. (`contains_edges` e `node` como helpers do módulo de teste.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codegraph-parser runner_calls`
Expected: FAIL.

- [ ] **Step 3: Implement runner nodes and their CONTAINS parent**

No coletor: se `test_file` e o nome base (`call_name`, ou o objeto de `x.<sufixo>` quando o
sufixo é modificador) é runner, criar o nó, empilhá-lo como função atual para a subárvore da
chamada e não gravar `Calls`. Em `shape_for_crg`, o pai de `CONTAINS` de um runner é escolhido
por extensão entre nós `Function` (incluindo runners), o mais interno.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codegraph-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codegraph-parser/src/languages/
git commit -m "feat(parser): vitest/jest runner calls as CRG Test nodes"
```

### Task 4: INHERITS no extrator e como texto no indexer

**Files:**
- Modify: `crates/codegraph-parser/src/languages/javascript.rs`
- Modify: `crates/codegraph-mcp/src/indexer.rs` (laço de resolução ~l.1174; `GRAPH_SHAPE_VERSION` l.44)
- Test: `javascript.rs` (extrator) e `crates/codegraph-mcp/tests/reconciliation_test.rs` (indexer;
  seguir o padrão dos testes de `a9aa9ba` que indexam um diretório temporário e leem o `graph.db`)

**Interfaces:**
- Consumes: Task 1.
- Produces: arestas `EdgeType::Extends` do nó da classe/interface para o texto da base, uma por
  base, para `extends` e `implements` (o CRG grava ambos como `INHERITS`). No indexer, `Extends`
  sem a chave `analyzer` não passa pelo catálogo: vira `CodeEdge::unresolved` com o texto, como
  `Imports` faz hoje.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn heritage_clauses_become_extends_edges() {
    let r = extract_ts(
        "class A extends B<T> implements I, ns.J {}\ninterface K extends L, M {}\n",
        "m.ts",
    );
    let mut got: Vec<(String, String)> = r.edges.iter()
        .filter(|e| e.edge_type == EdgeType::Extends)
        .map(|e| (qn_of(&r, e.from), e.to.clone()))
        .collect();
    got.sort();
    assert_eq!(got, vec![
        ("m.ts::A".into(), "B".into()), ("m.ts::A".into(), "I".into()),
        ("m.ts::K".into(), "L".into()), ("m.ts::K".into(), "M".into()),
    ]);
}
```

(`ns.J` é `nested_type_identifier`, que o CRG não lê no topo da cláusula: nenhuma aresta,
igual ao CRG. `T` de `type_arguments` também não.)

Teste de indexer `inherits_targets_stay_text`: projeto temporário com
`base.ts` = `export class Base {}\nexport class Leaf extends Base {}\n`; depois de indexar,
`SELECT target_qualified FROM edges WHERE kind='INHERITS'` devolve exatamente `["Base"]`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codegraph-parser heritage && cargo test -p codegraph-mcp --test reconciliation_test inherits`
Expected: FAIL.

- [ ] **Step 3: Implement heritage extraction, text targets and bump the shape version**

`GRAPH_SHAPE_VERSION = "crg-shape-2"`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codegraph-parser && cargo test -p codegraph-mcp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codegraph-parser/src/languages/javascript.rs crates/codegraph-mcp/src/indexer.rs crates/codegraph-mcp/tests/reconciliation_test.rs
git commit -m "feat(index): TS INHERITS edges kept as text like CRG"
```

### Task 5: Medir, atualizar a linha de base e o PLANO

**Files:**
- Modify: `docs/parity/baseline.md` (regerado)
- Modify: `PLANO.md` (tabela Jaccard, item 1 de "Próximos planos", riscos aceitos)

- [ ] **Step 1: Rodar a suíte e o harness**

```bash
cargo test --workspace --no-fail-fast -- --skip test_file_change_detection --skip test_file_monitoring_only --skip test_integration_test_suite
cargo build -p codegraph-mcp-server --bin codegraph
python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md docs/parity/baseline.md
```

Expected: suíte verde (exceto os 2 testes de watcher do WSL já conhecidos em
`codegraph-core::watch`); corpus `typescript` com Jaccard 1.000 em nós, CALLS, CONTAINS,
IMPORTS_FROM, INHERITS e TESTED_BY; REFERENCES continua 0. As outras linguagens não pioram
em relação a `docs/parity/baseline.md` atual (comparar a tabela antes de sobrescrever). Se
alguma aresta TS divergir, corrigir na task responsável antes de seguir.

- [ ] **Step 2: Atualizar `PLANO.md`**

Linha `typescript` da tabela com os valores novos; item 1 de "Próximos planos" marcado como
feito com o commit; os dois riscos aceitos da seção Review Focus na lista de dívida.

- [ ] **Step 3: Commit**

```bash
git add docs/parity/baseline.md PLANO.md
git commit -m "docs(parity): TypeScript baseline after extractor parity"
```
