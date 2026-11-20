# Fase 2: harness de paridade + forma do grafo CRG — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** medir a diferença entre o grafo do code-review-graph (CRG) e o do binário Rust no
mesmo repositório, e fechar as diferenças estruturais que valem para todas as linguagens
(nó `File`, `CONTAINS`, nomes qualificados com a classe, `Test`/`TESTED_BY`, imports como
arestas, alvos não resolvidos), antes do trabalho linguagem a linguagem.

**Architecture:** O harness é Python só com stdlib em `scripts/parity/`: copia o repositório
para dois diretórios, indexa um com o CRG da fonte (`../code-review-graph`, v2.4.0, schema 15)
e outro com `codegraph index`, e compara os dois `graph.db` por chaves relativas à raiz. No
Rust, uma etapa nova e pura em `codegraph-parser` (`crg_shape.rs`) reescreve o
`ExtractionResult` de cada arquivo no formato do CRG, chamada de `extract_for_language_with_policy`;
o que depende do banco inteiro (`TESTED_BY`, alvos não resolvidos) fica no `sqlite_store`.

**Tech Stack:** Python 3 stdlib (`sqlite3`, `argparse`, `json`, `unittest`), `uv`, Rust
(`codegraph-parser`, `codegraph-graph`, `codegraph-mcp`), `regex` já usado no workspace.

**Spec:** `PLANO.md`, Fase 2. Referência: `../code-review-graph` no commit `bbdda4b`
(`code_review_graph/parser.py`: `NodeInfo`/`EdgeInfo` l.1175–1213, `_TEST_PATTERNS` l.1793,
`is_test_file` l.2651, `_is_test_function` l.2701; arestas `TESTED_BY` l.4391 e seguintes).

## Global Constraints

- CRG de referência roda da fonte: `uv run --frozen --project <crg> code-review-graph build`.
  O `code-review-graph` instalado no PATH é 2.3.7 e gera schema 9; nunca usá-lo no harness.
- Os dois índices nunca dividem diretório: ambos gravam `<repo>/.code-review-graph/graph.db`.
- Harness só com stdlib do Python ≥ 3.10; nada de `pip install`.
- Chave de nó: `qualified_name` com o prefixo `<raiz>/` removido. Chave de aresta:
  `(kind, source, target)` com o mesmo prefixo removido.
- Concordância = |chaves em ambos| / |união das chaves| (Jaccard), por linguagem e por tipo.
- Critério da fase (PLANO.md): ≥95% de nós e ≥90% de arestas CALLS/IMPORTS_FROM nas
  linguagens prioritárias (Python, TS/JS, Go, Rust, Java). Este plano não fecha o critério;
  entrega o harness, a linha de base e a camada comum. Cada linguagem vira um plano próprio.
- IDs SHA de nós (`cg_nodes.uuid`) e o schema `schema_v15.sql` não mudam.
- Arquivos novos começam com duas linhas `ABOUTME:` (`#` em Python/shell, `//` em Rust).

## Forma do CRG (medida no piloto, fixture `sample_python.py`)

- Um nó `File` por arquivo: `kind=File`, `name=qualified_name=<abs path>`, linhas 1..último.
- Método: `<path>::Classe.metodo`, `parent_name=Classe`. Função dentro de função não ganha
  prefixo (`<path>::wrapper`).
- `CONTAINS`: `File → nó de topo` (incluindo funções aninhadas em funções) e
  `Classe → membro`. Todo nó que não é `File` tem exatamente um `CONTAINS` de entrada.
- Imports: sem nó; aresta `IMPORTS_FROM <path> → <módulo>` (alvo não resolvido = texto).
- Chamadas não resolvidas ficam: `CALLS <fonte> → print`.
- Testes: `kind=Test`, `is_test=1`. Para cada `CALLS t → x` com `t` do tipo `Test`, o CRG grava
  `TESTED_BY x → t` com a mesma linha, inclusive para `x` não resolvido (`assert`).
- `INHERITS Classe → Base` (texto, não resolvido quando a base é externa).

Piloto (7 arquivos de `tests/fixtures`): CRG 90 nós / 203 arestas; Rust 96 / 29. Rust não
tem `File`, `CONTAINS`, `TESTED_BY`, `INHERITS`, `REFERENCES`, nem `Test`; grava métodos sem
classe (`__init__@L10` por colisão), imports como nós `Import` com `IMPORTS_FROM` em laço
(`::os → ::os`) e descarta chamadas não resolvidas.

## Review Focus

1. Repositório com `.code-review-graph/` já existente (schema 9 do CRG instalado): o harness
   apaga `.code-review-graph/` nas cópias antes de indexar. Teste em Task 1.
2. Dois métodos homônimos em classes diferentes no mesmo arquivo: nomes distintos
   (`A.run`, `B.run`), sem sufixo `@L`. Teste em Task 3.
3. Arquivo sem nenhum símbolo (só imports ou vazio): ainda gera nó `File`. Teste em Task 3.
4. Alvo não resolvido igual ao nome de um nó de outro arquivo: o banco grava o texto, o
   resolvedor não inventa vínculo. Teste em Task 5.
5. Repositório com diretório-pai chamado `test` (`/tmp/test/repo`): só caminhos relativos à
   raiz contam para `is_test_file`. Teste em Task 4.

---

### Task 1: comparador de `graph.db`

**Files:**
- Create: `scripts/parity/parity.py`
- Create: `scripts/parity/test_parity.py`

**Interfaces:**
- Produces:
  - `load_graph(db: Path, root: str) -> Graph`; `Graph` é dataclass com
    `nodes: dict[str, NodeRow]` (chave = qn relativo) e `edges: set[EdgeKey]`.
    `NodeRow(kind, file, line_start, line_end, is_test, language)`;
    `EdgeKey = tuple[str, str, str]` (kind, source, target), tudo relativo.
  - `compare(crg: Graph, rust: Graph) -> Report` (dataclass serializável com `asdict`):
    `nodes_by_language[lang] = {"crg": n, "rust": n, "both": n, "jaccard": f,
    "only_crg": [..], "only_rust": [..], "divergent": [{qn, field, crg, rust}]}` e
    `edges_by_kind[kind] = {"crg", "rust", "both", "jaccard", "only_crg", "only_rust",
    "resolved_jaccard"}`. Listas cortadas em 50 itens, ordenadas.
  - `render_markdown(report: Report) -> str`.
  - CLI: `python3 scripts/parity/parity.py compare <crg.db> <crg_root> <rust.db> <rust_root>
    [--json out.json] [--md out.md]`.

- [ ] **Step 1: testes com bancos sintéticos** — helper `make_db(path, nodes, edges)` cria só
  as colunas usadas (`nodes(kind,name,qualified_name,file_path,line_start,line_end,language,
  is_test)`, `edges(kind,source_qualified,target_qualified,file_path,line)`).

```python
def test_relative_keys_match_across_roots(self):
    crg = make_db(..., root="/a/r1", nodes=[("Function", "/a/r1/m.py::f", "python", 1, 2, 0)])
    rust = make_db(..., root="/b/r2", nodes=[("Function", "/b/r2/m.py::f", "python", 1, 2, 0)])
    rep = compare(load_graph(crg, "/a/r1"), load_graph(rust, "/b/r2"))
    self.assertEqual(rep.nodes_by_language["python"]["jaccard"], 1.0)

def test_line_mismatch_is_divergent_not_missing(self):
    # mesmo qn, line_end 2 vs 3
    self.assertEqual(rep.nodes_by_language["python"]["both"], 1)
    self.assertEqual(rep.nodes_by_language["python"]["divergent"][0]["field"], "line_end")

def test_unresolved_targets_counted_separately(self):
    # CRG: CALLS m.py::f -> print ; Rust: nada
    e = rep.edges_by_kind["CALLS"]
    self.assertEqual((e["crg"], e["rust"], e["jaccard"]), (1, 0, 0.0))
    self.assertEqual(e["resolved_jaccard"], 1.0)   # nenhuma aresta resolvida em nenhum lado

def test_empty_union_is_full_agreement(self):
    self.assertEqual(compare(empty, empty).edges_by_kind, {})
```

- [ ] **Step 2:** `python3 -m unittest discover -s scripts/parity -v` → FAIL (`ModuleNotFoundError: parity`).
- [ ] **Step 3: implementar** `parity.py`. Divergência = mesmo qn com `kind`, `line_start`,
  `line_end` ou `is_test` diferentes (uma entrada por campo). Linguagem do nó `File` vem da
  coluna `language`; arestas não têm linguagem. Aresta "resolvida" = alvo é chave de nó no
  mesmo banco; `resolved_jaccard` compara só essas. Union vazia → jaccard 1.0.
- [ ] **Step 4:** mesmo comando → PASS.
- [ ] **Step 5:** `git add scripts/parity && git commit -m "feat(parity): compare CRG and Rust graph.db"`

### Task 2: runner, corpus e linha de base

**Files:**
- Modify: `scripts/parity/parity.py` (subcomando `run`)
- Create: `scripts/parity/corpus.json`
- Create: `scripts/parity/README.md`
- Create: `docs/parity/baseline.md` (gerado)
- Test: `scripts/parity/test_parity.py`

**Interfaces:**
- Consumes: `load_graph`, `compare`, `render_markdown` (Task 1).
- Produces:
  - `prepare_copies(src: Path, work: Path) -> tuple[Path, Path]`: copia `src` para
    `work/crg` e `work/rust` (sem `.git`, `target`, `node_modules`, `.code-review-graph`),
    roda `git init` + commit em cada (o CRG usa git para listar arquivos).
  - CLI: `parity.py run <src_dir> --work <dir> [--crg-repo ../code-review-graph]
    [--codegraph target/debug/codegraph] [--md out.md] [--json out.json]`.
    Também `parity.py corpus --work <dir> --md docs/parity/baseline.md`, que monta um
    repositório por linguagem a partir de `corpus.json` e concatena os relatórios.
  - `corpus.json`: `{"crg_commit": "bbdda4b", "languages": {"python": ["sample_python.py",
    "test_sample.py", "caller_example.py", "multi_call_example.py",
    "sample_callback_refs.py"], "typescript": ["sample_typescript.ts", "alias_importer.ts",
    "sample_vitest.test.ts"], "go": ["sample_go.go", "sample_dead_guard.go"], "rust":
    ["sample_rust.rs", "sample_dead_guard.rs", "sample_dead_guard_impl.rs"], "java":
    ["SampleJava.java", "SpringDI.java"]}}` (caminhos relativos a `tests/fixtures` do CRG).

- [ ] **Step 1: teste**

```python
def test_prepare_copies_drops_existing_index(self):
    # src contém .code-review-graph/graph.db e .git/
    crg, rust = prepare_copies(src, work)
    self.assertFalse((crg / ".code-review-graph").exists())
    self.assertTrue((rust / ".git").is_dir())        # repo novo, não o copiado
    self.assertEqual(sorted(p.name for p in crg.iterdir() if p.name != ".git"), ["m.py"])
```

- [ ] **Step 2:** rodar → FAIL. **Step 3:** implementar `prepare_copies` e `run`: CRG com
  `uv run --frozen --project <crg_repo> code-review-graph build` (cwd = cópia), Rust com
  `<codegraph> index . -r` (cwd = cópia); timeout 600 s cada; falha de qualquer um aborta com
  stderr do processo. Se `metadata.schema_version` do banco do CRG ≠ `15`, abortar com
  mensagem que cita o CRG instalado 2.3.7. **Step 4:** PASS.
- [ ] **Step 5: linha de base.** `cargo build -p codegraph-mcp-server --bin codegraph`, depois
  `python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md docs/parity/baseline.md`.
  Esperado: relatório com as 5 linguagens; Python perto de 96 vs 90 nós do piloto.
- [ ] **Step 6:** README com os dois comandos acima e os pré-requisitos (`uv`, CRG em
  `../code-review-graph`). Commit `feat(parity): runner, corpus and baseline report`.

### Task 3: `crg_shape` — `File`, `CONTAINS`, nome qualificado com classe

**Files:**
- Create: `crates/codegraph-parser/src/languages/crg_shape.rs`
- Modify: `crates/codegraph-parser/src/languages/mod.rs` (declarar módulo; chamar em
  `extract_for_language_with_policy`, l.85)
- Test: testes unitários em `crg_shape.rs`

**Interfaces:**
- Produces: `pub fn shape_for_crg(result: &mut ExtractionResult, file_path: &str,
  language: Language, line_count: u32)`. Efeitos:
  - insere nó `NodeType::Other("File".into())`, `name = file_path`, linhas `1..=line_count`,
    atributo `qualified_name = file_path`;
  - para cada nó `Function`/`Class`/`Struct`/`Trait`/`Interface`/`Enum`/`Type`, a classe
    envolvente mais interna por span (só `Class|Struct|Trait|Interface|Enum`) vira
    `parent_name` e o atributo `qualified_name` vira `"{file_path}::{Parent}.{name}"`
    (classes aninhadas: `Outer.Inner`); sem classe envolvente: `"{file_path}::{name}"`;
  - `EdgeType::Contains` de `File` (ou da classe envolvente) para cada um desses nós.
- `mapping::symbol` (codegraph-graph) já usa o atributo `qualified_name`; o writer grava
  `parent_name` a partir do atributo `parent_name` — confirmar em `write.rs` l.160–190 e
  ligar se faltar.

- [ ] **Step 1: testes** (Python via `extract_for_language`, como os testes existentes):

```rust
#[test] fn methods_get_class_prefix_and_contains() {
    // class A: def run  /  class B: def run  /  def top
    let qns = qualified_names(&r);
    assert!(qns.contains("m.py::A.run") && qns.contains("m.py::B.run"));
    assert_eq!(contains_edges(&r), set![("m.py","m.py::A"), ("m.py","m.py::B"),
        ("m.py","m.py::top"), ("m.py::A","m.py::A.run"), ("m.py::B","m.py::B.run")]);
}
#[test] fn nested_function_is_child_of_file() { /* def outer: def inner → m.py::inner, CONTAINS m.py→inner */ }
#[test] fn empty_file_still_has_file_node() { /* "" → um nó File, linhas 1..1 */ }
```

- [ ] **Step 2:** `cargo test -p codegraph-parser crg_shape` → FAIL.
- [ ] **Step 3:** implementar. `Contains` aponta por `to: String` = qualified name do filho;
  conferir que o resolvedor do indexer acha alvos por qualified name (`resolution.rs`).
- [ ] **Step 4:** PASS; `cargo test -p codegraph-parser` e `-p codegraph-mcp` sem regressão.
- [ ] **Step 5:** rodar `parity.py corpus`, colar a tabela de nós no commit.
  Commit `feat(parser): CRG file nodes, CONTAINS and class-qualified names`.

### Task 4: imports como aresta e detecção de testes

**Files:**
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs`
- Create: `crates/codegraph-parser/src/languages/test_detection.rs`

**Interfaces:**
- Produces:
  - `pub fn is_test_file(rel_path: &str) -> bool` e
    `pub fn is_test_function(name: &str, rel_path: &str, decorators: &[&str]) -> bool`,
    portados de `parser.py` (`_TEST_PATTERNS`, `_TEST_NAME_PATTERNS`, `_TEST_DIR_PATTERNS`,
    `_SINGULAR_TEST_DIR_RE`, `_TEST_RUNNER_NAMES`, `_TEST_ANNOTATIONS`), sempre sobre caminho
    relativo à raiz. `_is_shipped_test_package` fica fora (retorna `false`); anotar no
    relatório.
  - Em `shape_for_crg`: nós `NodeType::Import` saem; cada aresta `Imports` passa a ter
    `from = File` e `to` = módulo. Função que passa em `is_test_function` vira
    `NodeType::Other("Test")` com atributo `is_test = "true"`.
  - `shape_for_crg` recebe `rel_path: &str` além de `file_path` (assinatura final:
    `shape_for_crg(result, file_path, rel_path, language, line_count)`).

- [ ] **Step 1: testes**

```rust
#[test] fn import_becomes_file_edge() { /* import os → sem nó Import; edge Imports File→"os" */ }
#[test] fn test_function_is_test_kind() { /* tests/test_m.py::test_x → Other("Test"), is_test */ }
#[test] fn parent_dir_named_test_is_ignored() {
    assert!(!is_test_file("src/app.py"));          // repo em /tmp/test/repo
    assert!(is_test_file("tests/app.py"));
    assert!(is_test_file("app_test.go"));
}
```

- [ ] **Step 2–4:** FAIL → implementar (crate `regex`) → PASS.
- [ ] **Step 5:** corpus; commit `feat(parser): CRG import edges and test detection`.

### Task 5: alvos não resolvidos e `TESTED_BY`

**Files:**
- Modify: `crates/codegraph-mcp/src/indexer.rs` (resolução de arestas, perto de l.1328)
- Modify: `crates/codegraph-graph/src/sqlite_store/write.rs` e `reconcile.rs` (finalize)
- Test: `crates/codegraph-graph/tests/sqlite_write_test.rs`,
  `crates/codegraph-mcp/tests/` (teste de indexação existente mais próximo)

**Interfaces:**
- Produces:
  - Aresta cujo alvo não resolve é gravada com `target_qualified = <texto do alvo>` (como o
    CRG), em vez de descartada; `stats.unresolved_edges` continua contando.
  - No finalize do `SqliteGraphStore`: apagar `TESTED_BY` do projeto e reinserir
    `INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line)
    SELECT 'TESTED_BY', e.target_qualified, e.source_qualified, e.file_path, e.line
    FROM edges e JOIN nodes n ON n.qualified_name = e.source_qualified
    WHERE e.kind = 'CALLS' AND n.kind = 'Test'`.
  - `GraphQuery` (`graph_query/`) ignora alvos sem nó ao montar o petgraph (conferir; hoje
    pode assumir que todo alvo existe).

- [ ] **Step 1: testes**

```rust
#[test] fn unresolved_call_is_kept_as_text() { /* f chama print → edge CALLS ::f → "print" */ }
#[test] fn unresolved_target_never_binds_to_other_file() {
    /* a.py chama helper() sem import; b.py define helper → alvo continua "helper" */ }
#[test] fn tested_by_mirrors_calls_from_tests() {
    /* test_x chama f e assert → TESTED_BY ::f→::test_x e assert→::test_x, mesma linha */ }
#[test] fn graph_tools_skip_dangling_targets() { /* get_reverse_dependencies não falha */ }
```

  O segundo teste pode conflitar com o resolvedor lexical atual (Flow 1, passo 4); se ele
  hoje vincula `helper` entre arquivos, comparar com o CRG no corpus antes de mudar e
  registrar a decisão no commit.
- [ ] **Step 2–4:** FAIL → implementar → PASS; `cargo test -p codegraph-graph -p codegraph-mcp
  -p codegraph-mcp-tools`.
- [ ] **Step 5:** corpus; commit `feat(index): keep unresolved targets, derive TESTED_BY`.

### Task 6: nova linha de base, docs e PLANO

**Files:**
- Modify: `docs/parity/baseline.md` (regenerar; manter a tabela antiga como "antes")
- Modify: `docs/architecture/storage-sqlite.md` (tabela de kinds: `File`, `Test`, `Import`
  sai; `CONTAINS`/`TESTED_BY` agora produzidos)
- Modify: `PLANO.md` (Fase 2: progresso e próximos planos por linguagem, ordenados pela pior
  concordância de CALLS/IMPORTS_FROM)

- [ ] **Step 1:** `cargo test --workspace --all-features --no-fail-fast` com os `--skip` de
  watcher de `PLANO.md`; `cargo check --workspace --all-targets --all-features`.
- [ ] **Step 2:** regenerar baseline; conferir que nenhuma linguagem piorou em nós.
- [ ] **Step 3:** atualizar docs/PLANO; commit `docs(parity): baseline after CRG graph shape`.
