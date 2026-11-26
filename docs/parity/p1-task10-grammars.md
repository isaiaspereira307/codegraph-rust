# P1 Task 10 — compatibilidade das gramáticas Kotlin/Dart

O workspace usa `tree-sitter 0.27.0` (`Cargo.toml:115`). Como as gramáticas `tree-sitter-*`
ligam a biblioteca nativa `tree-sitter`, só pode haver **uma** versão na árvore de dependências;
uma gramática que depende de `tree-sitter 0.20/0.22` entra em conflito com `links`.

## Medição (2026-11-25)

Crate temporária em `/tmp/opencode/p1-t10/`, `cargo check` com `tree-sitter = "0.27.0"`:

| gramática | versão | requisito declarado | resultado |
|---|---|---|---|
| `tree-sitter-kotlin` | 0.3.8 (máxima) | `tree-sitter >=0.21,<0.23` | **falha**: duas versões de `links = "tree-sitter"` |
| `tree-sitter-dart` | 0.2.0 (atual do workspace) | `tree-sitter-language ^0.1` | OK |
| `tree-sitter-dart` | 0.3.0 (máxima) | `tree-sitter-language ^0.1` | OK |
| `tree-sitter-kotlin-ng` | 1.1.0 | `tree-sitter-language ^0.1` | OK |
| `tree-sitter-kotlin-sg` | 0.4.1 | a verificar na implementação | candidato alternativo |
| `brokk-tree-sitter-kotlin` | 0.4.9 | a verificar na implementação | candidato alternativo |

As duas gramáticas escolhidas exportam `LANGUAGE: LanguageFn`, diretamente conversível para
`tree_sitter::Language` (teste em `/tmp/opencode/p1-t10/grammar-probe/src/main.rs`). Os arquivos
`tests/fixtures/sample.kt` e `sample.dart` do CRG são parseados sem erro.

## Decisão preliminar

O bloqueio registrado como R5 no ledger (`tree-sitter-kotlin`/`tree-sitter-dart` incompatíveis
com 0.20.10) **não se aplica mais**: a versão pinada de Dart já é compatível e Kotlin tem um
fork ativo compatível. Não é necessário escalar decisão ao usuário para manter o P1 aberto.
A implementação das sondas e dos resolvers está registrada nas seções seguintes deste plano;
este documento fica como evidência da checagem de Step 1.

Fontes: `/api/v1/crates/{tree-sitter-kotlin,tree-sitter-dart}/versions` e
`/dependencies` do crates.io, consultados em 2026-11-25; saída integral dos `cargo check` nos
diretórios temporários acima.

## Medição por linguagem (2026-11-26)

CRG `bbdda4b`, binário debug, `python3 scripts/parity/parity.py run
scripts/parity/probes/<linguagem>-imports`. Critério do plano: nós ≥ 0.95, IMPORTS_FROM ≥ 0.90,
CALLS ≥ 0.80.

| linguagem | commit | nós | CALLS | IMPORTS_FROM | CONTAINS | TESTED_BY | INHERITS |
|---|---|---|---|---|---|---|---|
| Swift | `4eee114` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 |
| C/C++ | `317a05c` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | — |
| C# | `376e01d` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | — |
| Ruby | `a615a6a` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | — |
| PHP | `14a9e0b` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | — |
| Kotlin | `22b2383` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 0.167 |
| Dart | `5ba9370` | 1.000 | 1.000 | 1.000 | 0.833 | — | 1.000 |

"—": o CRG não gera arestas desse tipo na sonda. Flows e ARI das comunidades: 1.000 onde o CRG
gera flows (Ruby, Kotlin).

### O que cada sonda cobre

- **PHP**: `namespace` (sem nó), `use` simples e em grupo com alias (`use Demo\{App, Missing as
  Gone}`; o alvo não resolvido fica como texto `Demo\Missing`), `require_once` literal (o
  `__DIR__ . '/x.php'` calculado é ignorado, como no CRG), receptor construído
  (`$x = new C(); $x->m()`), `Classe::m()` resolvido pelo caminho do namespace, `self::m()`,
  `new self()`, testes PHPUnit por nome `test*` em arquivo de teste e por `/** @test */`.
  Layout `Demo/` (namespace = diretório): com `composer.json` o CRG grava nós `JsonPath`, que o
  Rust não indexa; o PSR-4 do `composer.json` está coberto só por teste unitário.
- **Kotlin**: supertipo `Base()`, `object` com chamada `Registry.register`, receptor tipado
  por parâmetro e por `val x: T`, `@Test`, import não resolvido (teste em `src/test` importando
  `demo.App`, que o CRG também não resolve).
- **Dart**: `./helper.dart` relativo, `package:probe/...` via `pubspec.yaml`, `dart:math`
  (texto), `extends`/`with`, chamadas com `this.` e por nome solto ligadas via import.

### Divergências aceitas

- **Kotlin, INHERITS 0.167**: o `_get_bases` do CRG pega o `type_identifier` do nome da
  própria classe, então toda classe Kotlin "herda" do próprio nome (`App → App`). O Rust não
  reproduz essa aresta; grava só os supertipos reais (`Base()` igual ao CRG).
- **Dart, CONTAINS 0.833 e uma comunidade a mais**: vêm dos nós `YamlPath` que o CRG grava para
  o `pubspec.yaml`. O Rust não indexa YAML.
- **Dart, forma do grafo**: o CRG usa a gramática do UserNobody14 (`class_definition`,
  `selector`/`argument_part`), e as crates `tree-sitter-dart` 0.2/0.3 trazem a do nielsenko. O
  extrator reproduz o resultado do CRG, não a AST: função = só a assinatura (sem getters,
  setters ou construtores), chamada sempre a partir do `File`, `new`/`const`/cascata não são
  chamadas, import relativo sem `./` (`'helper.dart'`) fica como texto. Divergência conhecida:
  em `a?.b()` o CRG grava a chamada como `a` (o seletor `?.` não é reconhecido); o Rust grava
  `b`.
- **PHP**: o resolver em grafo do CRG desempata vários candidatos `Classe.m` por import
  (`disambiguate`); o Rust só liga quando há um único candidato, ou pelo arquivo resolvido do
  `Classe::`. Parâmetro tipado (`Helper $h`) não vira receptor, igual ao CRG. Rotas e eventos
  Laravel (`_extract_php_laravel_edges`) não foram portados.
- Ficam como antes: arestas CALLS repetidas entre os mesmos nós são gravadas uma vez (o CRG
  grava uma por linha), o que baixa a concordância de `line` no Dart para 0.900.
