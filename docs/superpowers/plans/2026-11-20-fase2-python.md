# Fase 2 / Python: paridade do extrator com o CRG — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** levar o corpus `python` do harness a Jaccard 1.0 em nós (sem divergentes), CALLS,
CONTAINS, IMPORTS_FROM, INHERITS, REFERENCES e TESTED_BY (hoje 1.00 / 0.55 / 1.00 / 1.00 / 0 /
0 / 0.50), e uma sonda Python nova (`scripts/parity/probes/python/`, hoje nós 0.96, CALLS 0.30,
CONTAINS 0.90, IMPORTS_FROM 0.87, INHERITS/REFERENCES 0, TESTED_BY 0.36) ao mesmo patamar.

**Architecture:** `crates/codegraph-parser/src/languages/python.rs` é reescrito no molde dos
extratores novos (Go/Java/Rust): um pré-passe no topo do arquivo (nomes definidos, imports
`from`, ligações de módulo, nomes `TYPE_CHECKING`), uma pilha de chamadores (função; corpo de
classe e decorators → `File`), pilha de classes (folha), nome de chamada e receptor do CRG,
guarda morta estática, `INHERITS` como texto e `REFERENCES` de valores. Chamadas cujo alvo
depende de outro arquivo levam um atributo novo (`crg_py_import`, "módulo|símbolo") que o
indexer resolve pelo `ModuleIndex`; chamadas com receptor desconhecido levam `crg_unbound`
(nunca ligam). Imports `from M import x` geram candidatos de submódulo que o indexer só mantém
quando viram arquivo. O `crg_shape` ganha `is_test` em todo nó de arquivo de teste (todas as
linguagens, como `parse_bytes` do CRG) e a regra de classe aninhada do Python.

**Tech Stack:** Rust, `tree-sitter-python`; harness Python de `scripts/parity/`.

**Spec:** `PLANO.md`, Fase 2, "Próximos planos por linguagem" 5. Referência:
`../code-review-graph` no commit `bbdda4b`, `code_review_graph/parser.py`: estática de
guarda `_python_static_truth` l.316, `TYPE_CHECKING` l.274-332, `visit_If` l.394,
`_python_unreachable_call_positions` l.534, `_TEST_ANNOTATIONS` l.1907, `_is_test_function`
l.2701, `is_test` por arquivo em `parse_bytes` l.4127, receptor tipado `_apply_typed_call_targets`
l.9100, `_collect_python_receiver_evidence` l.9180, `_apply_python_receiver_evidence` l.10577,
`_extract_classes` l.16587-16818 (classe aninhada: CONTAINS do File e escopo-folha l.16747,
l.16802; ambiguidade l.16821), `_extract_functions` l.16948-17172, `_extract_calls` l.17298,
caller l.17405, receptor l.17444/17533, `_get_member_call_receiver_method` l.17681,
`_extract_value_references` l.19090-19302, `_emit_reference_if_known` l.19159,
`_collect_file_scope` l.19968, `_python_from_import_targets` l.20725,
`_python_folder_hides_module` l.20824, `_resolve_module_to_file` (Python) l.21404,
`_resolve_call_target` l.22648, `_resolve_imported_symbol` l.22688, `_get_bases` l.23805,
`_extract_import` l.24267, `_get_call_name` l.24725; `graph.py` `_receiver_backed_candidates`
l.2968.

## Global Constraints

- Mesmas dos planos anteriores: CRG da fonte via `uv run --frozen`, IDs SHA e `schema_v15.sql`
  intocados, arquivos novos com duas linhas `ABOUTME:`.
- `GRAPH_SHAPE_VERSION` (`crates/codegraph-mcp/src/indexer.rs:44`) passa de `"crg-shape-5"`
  para `"crg-shape-6"` (Task 1).
- IDs de nó continuam derivados do `qualified_name` pré-shape (`crg_shape::id_qualified_name`).
- As outras linguagens do corpus não podem regredir (a Task 1 mexe no `crg_shape` comum).
- Fora do escopo (fica registrado no PLANO): `line` das arestas; reexportação de pacote
  (`_python_reexport_origin`) e shims `sys.modules`; reescrita pós-build de módulo solto pelo
  sufixo (`python_resolver.py`); receptor tipado por anotação/atribuição
  (`_apply_typed_call_targets`, exceto `X().m()`); `REFERENCES` para arquivo de módulo
  (`register(util)`); ligação de nome solto não importado só no mesmo arquivo (o resolvedor
  atual também aceita arquivos importados); escopo-folha de classe aninhada em C++; checagem de
  sombreamento de `TYPE_CHECKING` além de atribuição no topo.

## Forma do CRG para Python (medida no corpus e nas sondas)

Nós e CONTAINS:
- `class_definition` → `Class`; `function_definition` (também `async def`) → `Function`;
  `decorated_definition` só embrulha. Função aninhada em função fica `F::inner` (do `File`);
  dentro de método, `F::Cls.inner` (de `F::Cls`) — o `crg_shape` já faz.
- **Classe aninhada em classe (só Python):** o nó é `F::Outer.Inner` com CONTAINS do `File`;
  dentro dela o escopo é só a folha: métodos `F::Inner.go`, CONTAINS de `F::Inner` (sem nó);
  classe mais funda `F::Inner.Deep` do `File`. Exceção: se duas classes do arquivo têm a mesma
  folha em escopos diferentes, os membros usam a cadeia completa (`F::Outer.Inner.go`, parent
  `Outer.Inner`).
- `is_test = 1` em **todo** nó de arquivo de teste (classes, `setUp`, fixtures), sem mudar o
  kind; `Test` continua só para funções por nome (`^test_`, `^Test`, …) ou decorator exato em
  `_TEST_ANNOTATIONS` (`@pytest.fixture`/`@pytest.mark.parametrize(...)` não contam).
- `decorators` = texto de cada `decorator` sem `@`, na ordem do código.

Chamadas:
- Nome: primeiro filho do `call`: `identifier` → texto; `attribute` → `attribute` (identificador
  mais à direita); outro (chamada, subscrito, parênteses) → sem aresta, filhos visitados.
- Chamador: `def` mais interna (lambda, comprehension não contam; defaults de parâmetro são da
  própria função); corpo de classe fora de método, decorators (de função de topo e de método) e
  módulo → `File`. Decorator de `def` aninhada em função → a função externa.
- Receptor (objeto do `attribute`):
  - `self`/`cls` dentro de classe `C` (folha) → `F::C.m` (o settle do indexer vira `m` sem nó).
  - identificador que é classe definida no topo do arquivo → `F::C.m`.
  - identificador ligado por `from M import X [as Y]` → alvo `m` com `crg_py_import = "M|X"`:
    o indexer tenta `M.X` como módulo (→ função de topo `m` desse arquivo), depois `M` como
    arquivo (→ `<arquivo>::X.m`).
  - identificador ligado por `import a.b as c` → `crg_py_import = "a.b|"` (módulo).
  - `X()` com `X` classe local ou nome importado → como se o receptor fosse `X`.
  - qualquer outro (`x`, `os.path`, `self.repo`, `super()`, `obj["k"]`, `b.Thing`) → `m` com
    `crg_unbound` (nunca liga, nem a função livre).
- Sem receptor: `F::nome` se `nome` está em `defined_names` (defs/classes de topo); se ligado
  por `from M import X [as nome]` → alvo `nome` com `crg_py_import = "M|X"` e sem `receiver`
  (o indexer liga a `<arquivo de M>::X`); senão o nome solto (resolvedor atual: mesmo arquivo,
  inclusive métodos, ou arquivo importado). O indexer distingue receptor pela chave `receiver`
  do metadado da aresta.
- Guarda morta: chamada dentro do corpo de `if` cuja condição é estaticamente falsa (ou do
  `else`/`elif` de condição verdadeira) não gera aresta. Estático: `True`/`False`/inteiros,
  `not`, `and`/`or`, parênteses, `TYPE_CHECKING` importado `from typing` (ou alias) ou
  `typing.TYPE_CHECKING` com `import typing`, desde que o nome não seja atribuído no topo.
  `while False`, strings e `None` continuam vivos.

INHERITS: bases `identifier`/`attribute` da `argument_list` da classe, texto cru
(`unittest.TestCase`); `keyword_argument` (`metaclass=`) e subscritos ficam fora; origem = a
classe; nunca resolvido (o indexer já guarda `Extends` como texto).

REFERENCES (`_extract_value_references`), só `identifier` puro em: filhos de `argument_list`
e valor de `keyword_argument` (inclui a lista de bases da classe → origem `File`), elementos de
`list`, valores de `pair` de dict, lado direito de atribuição cujo lado esquerdo é `attribute`
ou `subscript`. Origem: função envolvente, senão `File` (corpo de classe → `File`). Filtro:
fora `self`, `cls`, `super`, `None`, `True`, `False`, nomes TODO-MAIÚSCULOS e de um caractere;
só nomes em `defined_names` (alvo `F::nome`) ou importados por `from` (alvo como nas chamadas).
Tuplas, sets, `return f` e `h = f` não geram nada.

Imports (alvo de `IMPORTS_FROM`, origem `File`):
- `import a.b`, `import a.b as c`, `import a, b` → um alvo por módulo (`a.b`).
- `from M import x, y` → `M` mais candidatos `M.x`, `M.y` (`.` + `x` = `.x`) que só ficam se
  resolvem para arquivo; se `M` não resolve mas algum candidato sim (pacote namespace), `M` sai.
- `from M import *` → `M`.
- Resolução (já no `ModuleIndex`): relativo sobe um nível por ponto extra; absoluto sobe da
  pasta do chamador, `__init__.py` antes de `.py`. Novo: `_python_folder_hides_module` — numa
  pasta com `__init__.py`, módulo cujo primeiro segmento é da stdlib não é procurado ali.

## Review Focus

1. Classe aninhada: `F::Child.Inner` do `File`, `F::Inner.go` de `F::Inner`; com folha
   ambígua, cadeia completa — teste na Task 1.
2. `self.step()` em subclasse sem `step` vira `step` (settle), não `Base.step` — Task 2 e
   harness.
3. `x.save()` com `Repo.save` num arquivo importado não liga (`crg_unbound`) — Task 2/Task 4.
4. Chamadas depois de uma função (decorator seguinte, `if __name__ == "__main__":`) saem do
   `File`, não da função anterior (bug atual de `current_function_id`) — Task 2.
5. `from app import util` gera `app/util.py` (e `app/__init__.py` se existir); `util.helper()`
   liga a `app/util.py::helper` — Task 4.

---

### Task 1: `crg_shape` — `is_test` por arquivo e classe aninhada do Python

**Files:**
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs` (`shape_for_crg`)
- Modify: `crates/codegraph-mcp/src/indexer.rs:44` (`GRAPH_SHAPE_VERSION = "crg-shape-6"`)
- Test: `crates/codegraph-parser/src/languages/crg_shape.rs` (`mod tests`)

- [ ] **Step 1: testes que falham**

```rust
#[test]
fn every_node_of_a_test_file_is_flagged() {
    let source = "class TestA:\n    def setUp(self):\n        pass\n\ndef helper():\n    pass\n";
    // extract_for_language_at com rel "tests/test_m.py"
    // espera is_test = "true" em TestA, TestA.setUp e helper; kinds Class/Function.
}

#[test]
fn python_nested_classes_scope_members_by_leaf() {
    let r = extract_python("class Child:\n    class Inner:\n        def go(self):\n            pass\n        class Deep:\n            pass\n");
    // qualified: m.py::Child, m.py::Child.Inner, m.py::Inner.go, m.py::Inner.Deep
    // contains: (m.py, m.py::Child), (m.py, m.py::Child.Inner), (m.py::Inner, m.py::Inner.go),
    //           (m.py, m.py::Inner.Deep)
}

#[test]
fn python_ambiguous_leaf_keeps_the_full_chain() {
    let r = extract_python("class A:\n    class In:\n        def go(self): pass\nclass B:\n    class In:\n        def go(self): pass\n");
    // m.py::A.In.go (parent "A.In", contains de m.py::A.In) e m.py::B.In.go
}
```

O teste `nested_classes_are_dotted` existente muda para a forma nova (`m.py::Inner.go`).

- [ ] **Step 2:** `cargo test -p codegraph-parser languages::crg_shape` → FAIL.

- [ ] **Step 3: implementar**
  - Depois do loop que marca `Test`: se `test_detection::is_test_file(rel_path)`, inserir
    `is_test = "true"` em todo membro (sem mudar kind nem `PRE_SHAPE_KIND_ATTR`).
  - Python (`language == Language::Python`): ao calcular `paths`, classe com pai classe tem
    path `Pai.Nome` (cadeia completa) mas CONTAINS do `File`; o "escopo" usado pelos filhos
    é só a folha (`Nome`), salvo se a folha é ambígua (duas classes do arquivo com a mesma
    folha e cadeias diferentes) — aí a cadeia completa. Membro de classe aninhada: qualified
    `F::<escopo>.<nome>`, `parent_name` = escopo, CONTAINS de `F::<escopo>` (por texto; o
    `from` é o id da classe-pai, que já é o nó certo — só o texto `to` e o `parent_name`
    mudam). Classes de topo e outras linguagens ficam como estão.
  - `GRAPH_SHAPE_VERSION = "crg-shape-6"`.

- [ ] **Step 4:** `cargo test -p codegraph-parser` → PASS.

- [ ] **Step 5: Commit** `feat(parser): CRG test-file flags and Python nested class scopes`

### Task 2: extrator Python — nós, chamadas, receptores, guarda morta, INHERITS

**Files:**
- Modify (reescrita): `crates/codegraph-parser/src/languages/python.rs`
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs` (constantes `UNBOUND_ATTR =
  "crg_unbound"`, `PY_IMPORT_ATTR = "crg_py_import"`)
- Test: `crates/codegraph-parser/src/languages/python.rs` (`mod tests`, helpers no molde de
  `go.rs`: `extract_py(source, path)`, `edges(&r, kind) -> BTreeSet<(String, String)>` com
  qualified da origem e `to`, `edge_attr(&r, kind, to, attr)`)

**Interfaces:**
- Produces: `struct FileScope { defined: HashSet<String>, classes: HashSet<String>,
  from_imports: HashMap<String, (String, String)> /* local -> (módulo, nome original) */,
  module_aliases: HashMap<String, String> /* import a.b as c: c -> a.b */,
  type_checking: HashSet<String> /* nomes que valem TYPE_CHECKING */, typing_modules:
  HashSet<String> }`, calculado no pré-passe sobre os filhos diretos da raiz; a Task 3 usa o
  mesmo `FileScope` para REFERENCES.

- [ ] **Step 1: testes que falham** (um por regra da seção de forma):

```rust
#[test]
fn call_names_and_callers_follow_crg() {
    let r = extract_py(
        "import json as js\nCONST = helper(1)\n\nclass Base:\n    x = make()\n    def step(self):\n        return 1\n\n\
         class Child(Base):\n    @staticmethod\n    @deco.wrap(1)\n    def run(self):\n        self.step()\n        Base.step(self)\n        js.loads('x')\n        self.repo.save()\n        super().__init__()\n        get()()\n        obj['k'].m()\n        f = lambda: helper(2)\n        return [v.upper() for v in xs]\n\n\
         def outer():\n    def inner():\n        pass\n    return inner()\n\nif __name__ == '__main__':\n    main()\n",
        "m.py",
    );
    assert_eq!(edges(&r, EdgeType::Calls), set(&[
        ("m.py", "helper"), ("m.py", "make"), ("m.py", "wrap"), ("m.py", "main"),
        ("m.py::Child.run", "m.py::Child.step"), ("m.py::Child.run", "m.py::Base.step"),
        ("m.py::Child.run", "loads"), ("m.py::Child.run", "save"), ("m.py::Child.run", "super"),
        ("m.py::Child.run", "__init__"), ("m.py::Child.run", "get"), ("m.py::Child.run", "m"),
        ("m.py::Child.run", "helper"), ("m.py::Child.run", "upper"),
        ("m.py::outer", "m.py::inner"),
    ]));
    assert!(edge_attr(&r, EdgeType::Calls, "save", "crg_unbound").is_some());
    assert!(edge_attr(&r, EdgeType::Calls, "loads", "crg_unbound").is_some());
}

#[test]
fn imported_receivers_carry_the_import() {
    let r = extract_py(
        "from pkg import b\nfrom pkg.b import Thing, helper as hh\nimport pkg.b as bm\n\n\
         def f():\n    b.helper()\n    bm.helper()\n    hh()\n    Thing.go()\n    Thing().go()\n    b.Thing.go()\n",
        "pkg/a.py",
    );
    // b.helper -> to "helper", crg_py_import "pkg|b"; bm.helper -> "helper", "pkg.b|";
    // hh() -> to "hh", "pkg.b|helper"; Thing.go -> "go", "pkg.b|Thing"; Thing() -> "Thing",
    // "pkg.b|Thing"; Thing().go -> "go", "pkg.b|Thing"; b.Thing.go -> "go" com crg_unbound.
}

#[test]
fn static_dead_guards_drop_calls() {
    let r = extract_py(
        "from typing import TYPE_CHECKING\n\ndef g():\n    if False:\n        a()\n    if not True:\n        b()\n    if TYPE_CHECKING:\n        c()\n    if 0:\n        d()\n    else:\n        e()\n    if True:\n        f()\n    else:\n        h()\n    while False:\n        i()\n    if x:\n        j()\n",
        "m.py",
    );
    // CALLS de m.py::g: e, f, i, j
}

#[test]
fn nested_method_calls_and_cls() {
    let r = extract_py(
        "class K:\n    @classmethod\n    def make(cls):\n        return cls.build()\n    def build(self):\n        def nested():\n            return self.make()\n        return nested()\n",
        "m.py",
    );
    // (K.make, m.py::K.build), (K.nested, m.py::K.make), (K.build, m.py::K.nested)
}

#[test]
fn bases_are_inherits_text_and_decorators_recorded() {
    let r = extract_py(
        "import unittest\nclass T(Base, unittest.TestCase, Generic[X], metaclass=M):\n    pass\n\n@pytest.mark.parametrize('x', [1])\n@dec\ndef p():\n    pass\n",
        "m.py",
    );
    // Extends: (m.py::T, Base), (m.py::T, unittest.TestCase)
    // decorators de p = "pytest.mark.parametrize('x', [1]),dec"
}
```

- [ ] **Step 2:** `cargo test -p codegraph-parser languages::python` → FAIL.

- [ ] **Step 3: implementar**
  - Pré-passe `FileScope` nos filhos diretos da raiz (desembrulhando `decorated_definition`):
    `defined` (defs e classes), `classes`, `from_imports` (`import_from_statement`: para cada
    `dotted_name`/`aliased_import` importado, local → (módulo como escrito, incluindo pontos,
    nome original)), `module_aliases` (`import_statement` com `aliased_import`), nomes
    `TYPE_CHECKING` (de `from typing import TYPE_CHECKING [as X]`; `import typing [as t]` → aceitar
    `t.TYPE_CHECKING`), menos nomes atribuídos no topo.
  - Walk recursivo com `function_stack: Vec<Option<NodeId>>` e `class_stack: Vec<String>`
    (folha). `class_definition`: nó `Class`, Extends por base, push `None` na pilha de
    chamadores e a folha na de classes. `function_definition`: nó `Function` (complexidade e
    `decorators` do `decorated_definition` pai), push `Some(id)`. Decorators são filhos do
    `decorated_definition` visitados antes da definição, então caem no escopo externo.
  - `call`: se dentro de guarda morta (ancestral `if_statement` ou `elif_clause`/`else_clause`
    com condição estática no ramo morto) → não emite nem desce? O CRG não desce: não emitir a
    aresta e continuar visitando os filhos com a mesma checagem (todos os descendentes também
    estão no bloco morto). Nome/receptor/alvo pelas regras da seção de forma. Chamador
    `None`/vazio → `crg_shape::FILE_SCOPE_ATTR`, `from = NodeId::nil()`.
  - Apagar `extract_type_hints`, as `References` de base/anotação e `extract_python` stub só se
    nada mais usar (`grep -rn extract_python crates`).
  - Metadados de aresta: `source_file`, `receiver` quando houver, `PY_IMPORT_ATTR`/
    `UNBOUND_ATTR` conforme a regra.

- [ ] **Step 4:** `cargo test -p codegraph-parser` → PASS.

- [ ] **Step 5: Commit** `feat(parser): Python calls, receivers, dead guards and INHERITS like CRG`

### Task 3: REFERENCES de valores

**Files:**
- Modify: `crates/codegraph-parser/src/languages/python.rs`
- Test: idem

- [ ] **Step 1: teste que falha**

```rust
#[test]
fn value_references_follow_crg() {
    let r = extract_py(
        "from x import imp\n\nclass Base:\n    pass\n\nclass C(Base, metaclass=Base):\n    pass\n\n\
         def w():\n    pass\n\ndef use(items):\n    sorted(items, key=w)\n    run(w, cb=imp)\n    l = [w]\n    d = {'k': w}\n    obj.cb = w\n    h = w\n    t = (w,)\n    g(self, None, X, w2)\n    return w\n",
        "m.py",
    );
    // REFERENCES: (m.py, m.py::Base) e (m.py::use, m.py::w) e (m.py::use, imp) com
    // crg_py_import "x|imp"; nada para h/t/return/self/None/X/w2 (não definido).
}
```

- [ ] **Step 2:** FAIL. **Step 3:** implementar como na seção de forma (mesmo alvo das
  chamadas sem receptor; só nomes conhecidos). `ExtractionPolicy.references = false` desliga.
  **Step 4:** PASS.

- [ ] **Step 5: Commit** `feat(parser): Python value REFERENCES like CRG`

### Task 4: imports e ligação no indexer

**Files:**
- Modify: `crates/codegraph-parser/src/languages/python.rs` (imports)
- Modify: `crates/codegraph-mcp/src/module_index.rs` (`python`: stdlib escondida)
- Modify: `crates/codegraph-mcp/src/resolution.rs` (`py_import_target`)
- Modify: `crates/codegraph-mcp/src/indexer.rs` (`resolve_target`, laço de imports)
- Create: `crates/codegraph-mcp/src/python_stdlib.rs` (lista `sys.stdlib_module_names` do
  Python 3.12, gerada uma vez e colada; `ABOUTME:`)
- Test: `module_index.rs`, `resolution.rs`, `python.rs`, e `crates/codegraph-mcp/tests/` se já
  houver teste de indexação Python de ponta a ponta (`grep -rln "\.py" crates/codegraph-mcp/tests`).

- [ ] **Step 1: testes que falham**
  - extrator: `import json as js, os` → Imports `json`, `os`; `from app import util, x` →
    `app`, `app.util` e `app.x` (os dois últimos com `crg_py_submodule`); `from . import y` →
    `.` e `.y`; `from m import *` → `m`.
  - `module_index`: com `pkg/__init__.py` e `pkg/typing.py`, `resolve("pkg/a.py", "typing")`
    é `None`; `resolve("pkg/a.py", "b")` acha `pkg/b.py`.
  - `resolution`: `py_import_target` liga `helper` + `"pkg|b"` a `pkg/b.py::helper` (função
    de topo), `go` + `"pkg.b|Thing"` a `pkg/b.py::Thing.go`, `helper` + `"pkg.b|"` a
    `pkg/b.py::helper`; método homônimo não serve de função de topo.

- [ ] **Step 2:** FAIL.

- [ ] **Step 3: implementar**
  - Extrator: um `Imports` por módulo do `import_statement`; `import_from_statement` gera `M`
    e os candidatos marcados.
  - Indexer, laço de imports: candidato `crg_py_submodule` que não resolve para arquivo é
    descartado (nem texto); se o `M` da mesma instrução não resolve e algum candidato sim, `M`
    também sai (agrupar pelo `source_span`).
  - `resolve_target`: `UNBOUND_ATTR` → `(None, ("unbound", 1.0))`; `PY_IMPORT_ATTR` →
    `catalog.py_import_target(edge)`: resolve `M.X` (ou `M` quando o símbolo é vazio) como
    módulo → nó de topo (`!has_parent`) com qualified `<arquivo>::<to>`; senão `M` como
    arquivo → nó com qualified `<arquivo>::X.<to>` (receptor) ou `<arquivo>::X` (chamada sem
    receptor, `to` = alias local); sem ligação, o texto fica `to` (sem busca léxica).
  - `ModuleIndex::python`: ao testar candidatos num diretório que tem `__init__.py`, pular se
    o primeiro segmento do módulo está na stdlib.

- [ ] **Step 4:** `cargo test -p codegraph-mcp && cargo test -p codegraph-parser` → PASS.

- [ ] **Step 5: Commit** `feat(index): Python submodule imports and import-bound calls like CRG`

### Task 5: sonda, harness, linha de base, PLANO e limpeza

**Files:**
- Create: `scripts/parity/probes/python/` (cópia da sonda usada neste plano: `app/core.py`,
  `app/util.py`, `app/models.py`, `app/__init__.py`, `tests/test_core.py`, `pkg/a.py`,
  `pkg/b.py`, `pkg/__init__.py`) e uma linha no `scripts/parity/README.md` de como rodá-la.
- Modify: `docs/parity/baseline.md`, `PLANO.md`

- [ ] **Step 1:** `cargo build -p codegraph-mcp-server --bin codegraph && python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md docs/parity/baseline.md`
  Expected: `python` 1.000 em nós (0 divergentes), CALLS, CONTAINS, IMPORTS_FROM, INHERITS,
  REFERENCES, TESTED_BY; demais linguagens sem regressão.
- [ ] **Step 2:** `python3 scripts/parity/parity.py run scripts/parity/probes/python --work $TMPDIR/pyprobe --md $TMPDIR/pyprobe.md`
  Expected: 1.000 em tudo, ou cada divergência restante listada como fora do escopo.
- [ ] **Step 3:** divergências → voltar à task dona com teste novo e repetir.
- [ ] **Step 4:** suíte: `cargo test --workspace --all-features --no-fail-fast -- --skip test_file_change_detection --skip test_file_monitoring_only --skip test_integration_test_suite` e `cargo clippy -p codegraph-parser -p codegraph-mcp --all-targets --all-features`.
- [ ] **Step 5:** PLANO.md: item 5 ✅ com resumo, linha `python` da tabela e "Fica para depois".
- [ ] **Step 6: Commit** `docs(parity): Python baseline after extractor parity`
- [ ] **Step 7:** `cargo clean` e reportar o espaço liberado.
