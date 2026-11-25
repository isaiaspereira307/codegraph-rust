# P1 — Plano de paridade e qualidade (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fechar o item P1 da avaliação do `PLANO.md`: paridade de parsers/resolução, decisões de semântica pendentes e dívida de análise de grafo. Nenhum item P0 (release, tap, Action, Windows) ou P2 (Fase 5) entra aqui.

**Architecture:** Cada item é um port pontual do comportamento do CRG (`../code-review-graph/code_review_graph/parser.py`, `graph.py`, `flows.py`, `changes.py`) para o pipeline existente: extratores em `codegraph-parser/src/languages/`, resolução em `codegraph-mcp/src/{resolution,module_index,rust_crates}.rs`, persistência em `codegraph-graph/src/sqlite_store/write.rs` e consulta em `codegraph-graph/src/{graph_query,analysis}/`. O harness `scripts/parity/` mede antes/depois; cada tarefa só é fechada com número publicado em `docs/parity/`.

**Tech Stack:** Rust 2024 / tree-sitter / rusqlite / petgraph; harness de paridade em Python somente como ferramenta de desenvolvimento (o produto continua sem Python).

**Spec:** `PLANO.md` (Fases 2, 2.5, 3 e 4) + avaliação P1 que originou este plano. Tarefas grandes (ex.: uma linguagem nova) podem ser expandidas para plano próprio seguindo este mesmo modelo.

## Global Constraints

- Repositório `isaiaspereira307/codegraph-rust`, push direto na `main`, sem PR e sem upstream. Commits em Conventional Commits; cada tarefa termina em commit.
- Schema SQLite continua v15 em `.code-review-graph/graph.db`. Nenhuma tarefa altera o schema sem plano separado.
- Raiz canônica dos caminhos: `canonical_file_key` (`crates/codegraph-core/src/types.rs:50`) é a fonte da verdade para IDs; `qualified_name` deve seguir a mesma normalização (Task 2).
- `fast` continua o tier padrão. `balanced`/`full` só são avaliados na Task 9; mudança de padrão exige medição e decisão explícita.
- Determinismo sempre: seed fixa, ordem estável, sem não-determinismo em ID de flow, comunidade ou aresta. Nenhum resultado de ferramenta muda de forma sem atualizar `graph_tool_schemas.rs`, executor e testes.
- Todo port de comportamento do CRG começa com teste que falha e termina com medição no harness. **Se qualquer métrica já em 1.000 cair, a mudança é revertida ou limitada por flag interna.**
- Linha de base congelada em `docs/parity/p1-baseline.md` antes da primeira mudança (Task 1). Números sem comando reprodutível não valem.
- Ambiente local: suíte completa ignora os 5 testes de watcher do WSL; Clippy estrito tem diagnósticos antigos em `codegraph-core` — reportar avisos novos nos crates tocados, nunca chamar a suíte de "limpa" sem ressalva.
- Kotlin e Dart estão desligados em `crates/codegraph-parser/src/language.rs` e no `Cargo.toml` do parser por incompatibilidade de versão do tree-sitter (v0.20.10). A Task 10 precisa resolver isso ou declarar as duas linguagens como fora de alcance.

## Review Focus

- `qualified_name` com `..`, `./` ou caminho absoluto apontando para o mesmo nó; `id_qualified_name` não pode mudar junto.
- Receptor Python tipado por anotação que hoje cai em `crg_unbound` e perde a aresta CALLS.
- `pub use`/glob/`mod` com `#[path]`/`super::` acima da raiz em Rust; mudança só no `Cargo.toml` precisa re-resolver as fontes.
- TSX/JSX, `require()`, aliases npm/workspaces e `Foo.prototype.bar = …`; imports de diretório de pacote Go via `go.mod`/`replace`/`vendor`.
- Flows com ID estável entre builds sem mudança e `nodes.extra` preservado quando o banco foi escrito também pelo CRG.
- `update` de 2 arquivos: 70 s (debug) contra 18 s do CRG; medir release, debug e 3 repetições.
- Herança no `AnalysisGraph`: ferramentas de flows/comunidades/risco continuam ignorando `INHERITS`/`IMPLEMENTS`; `OVERRIDES` não existe.

---

### Task 1: Congelar a linha de base e o detector de regressão

Sem isso, nenhuma tarefa seguinte tem critério objetivo.

**Files:**
- Create: `scripts/parity/p1_snapshot.sh`, `scripts/parity/check_p1.py`
- Create: `docs/parity/p1-baseline.md`
- Modify: `scripts/parity/README.md`

**Interfaces:**
- Produces: `p1_snapshot.sh <out-dir>` roda corpus, 4 probes (`python`, `python-imports`, `rust-imports`, `inheritance`) e os 3 cenários de `scripts/parity/changes/`; grava JSON/MD em `<out-dir>` e imprime um resumo. Não escreve em `docs/`.
- Produces: `check_p1.py <snapshot-dir>` → exit 0/1; compara as métricas do snapshot com os limites do baseline e falha em qualquer queda.

- [ ] **Step 1:** Escrever `p1_snapshot.sh` reaproveitando os comandos de `scripts/parity/README.md:18-48` e `docs/parity/changes.md:9-22`. Rodar com `cargo build -p codegraph-mcp-server --bin codegraph` e `uv run --frozen --project ../code-review-graph`; saída em `/tmp/opencode/parity-p1/`.
- [ ] **Step 2:** Rodar o snapshot. Expected: termina sem erro; `docs/parity/p1-baseline.md` fica com a tabela de nós/CALLS/IMPORTS_FROM/line agreement/CONTAINS/INHERITS/TESTED_BY por linguagem, entry points, ARI, flows, risco por função e tempo de `update`.
- [ ] **Step 3:** Escrever `check_p1.py` com os limites atuais (python CALLS 0.923, rust-imports 1.000, 31/35 risco, etc.) e o teste `scripts/parity/test_parity.py` para o próprio comparador.
- [ ] **Step 4:** `python3 -m unittest discover -s scripts/parity -v` e `python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1`. Expected: verde.
- [ ] **Step 5:** Commit `test(parity): freeze P1 baseline and regression check`.

### Task 2: `qualified_name` canônico (pendente da Fase 2.5)

**Files:**
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs` (`shape_for_crg`, `default_rel_path`), `crates/codegraph-mcp/src/indexer.rs` (`GRAPH_SHAPE_VERSION`)
- Test: `crates/codegraph-parser/src/languages/crg_shape.rs` (`mod tests`)

**Interfaces:**
- Produces: nó `File` e prefixo de todo `qualified_name`/endpoint com o `rel_path` normalizado (sem `./`, `..` ou `//`), nunca o `file_path` cru. `id_qualified_name` (`crg_shape.rs:57`) continua devolvendo o valor pré-shape.

- [ ] **Step 1:** Teste que falha: extrair o mesmo fixture por `.`, caminho absoluto e `./../fx/.` e afirmar conjuntos idênticos de `qualified_name` e nenhum `..`.
- [ ] **Step 2:** Usar `rel_path` em `crg_shape.rs:240-343` (inclusive `source_file`) e normalizar `default_rel_path`; subir `GRAPH_SHAPE_VERSION` de `crg-shape-12` para `crg-shape-13` para forçar rebuild.
- [ ] **Step 3:** `cargo test -p codegraph-parser crg_shape` e `python3 scripts/parity/parity.py run scripts/parity/probes/python --work /tmp/opencode/p1-qn --md /tmp/opencode/p1-qn.md`. Expected: teste verde, nenhuma métrica cai.
- [ ] **Step 4:** Commit `fix(parser): canonicalize qualified_name across path forms`.

### Task 3: Python — receptor tipado e decisão sobre `REFERENCES` no tier `fast`

**Files:**
- Modify: `crates/codegraph-parser/src/languages/python.rs` (`receiver_target`, escopo de parâmetros/atribuições), `crates/codegraph-mcp/src/resolution.rs` se precisar de evidência de módulo
- Test: `crates/codegraph-parser/src/languages/python.rs` (`mod tests`); fixture `scripts/parity/probes/python/`

**Interfaces:**
- Consumes: `crg_py_import`/`crg_unbound` atuais e `_apply_python_receiver_evidence` do CRG (`parser.py:10577`).
- Produces: `service: AuthService; service.authenticate()` e `svc = AuthService(); svc.authenticate()` ligam a `AuthService.authenticate`; fallback `crg_unbound` preservado.

- [ ] **Step 1:** Teste que falha em `python.rs` para os dois casos acima, com `receiver` em `edges.metadata`.
- [ ] **Step 2:** Portar o mínimo de `_apply_python_receiver_evidence`: anotação de parâmetro/local, atribuição de construtor, caminho de módulo pontuado (`import q; q.lib.f()`) e construtor de módulo (`mod.Thing().go()`). Não portar reescrita pós-build (vai na Task 4).
- [ ] **Step 3:** Medir corpus + probe `python`: CALLS 0.923 → ≥ 0.95 e CALLS resolvidos 0.833 → ≥ 0.95, sem queda em nós/IMPORTS_FROM.
- [ ] **Step 4:** Decidir `REFERENCES` no `fast`: rodar corpus/probe com `--index-tier fast` e `--index-tier full`, comparar Jaccard e crescimento de arestas. Se as referências de valor não aumentarem o grafo em mais de 5% e mantiverem/elevarem a concordância, habilitar no `fast`; senão registrar a decisão em `docs/parity/baseline.md` e deixar como está.
- [ ] **Step 5:** Commit `fix(parser): resolve python typed receivers` (e, se aplicável, `feat(indexer): keep value references in fast tier`).

### Task 4: Python — reexportação de pacote e `sys.modules`

**Files:**
- Modify: `crates/codegraph-parser/src/languages/python.rs`, `crates/codegraph-mcp/src/module_index.rs`
- Test: probe `scripts/parity/probes/python-imports/`

**Interfaces:**
- Consumes: `_python_reexport_origin` (`parser.py:22720`) e os shims `sys.modules` do CRG.
- Produces: `from pkg import X` com `pkg/__init__.py` reexportando `X` liga ao arquivo de origem; aliases registrados em `sys.modules` resolvem.

- [ ] **Step 1:** Acrescentar ao probe `python-imports` um pacote com reexport e um `sys.modules` alias; rodar harness e registrar falha (CALLS 0.684).
- [ ] **Step 2:** Implementar a origem da reexportação no `ModuleIndex` e o alias de módulo no extrator; manter alvos de texto quando ambíguo.
- [ ] **Step 3:** Medir: CALLS do probe ≥ 0.90, IMPORTS_FROM e nós em 1.000; rodar `check_p1.py`.
- [ ] **Step 4:** Commit `fix(parser): resolve python package reexports and sys.modules aliases`.

### Task 5: Rust — sobras de resolução entre arquivos e custo por aresta

**Files:**
- Modify: `crates/codegraph-parser/src/languages/rust.rs`, `crates/codegraph-mcp/src/{module_index,rust_crates,resolution,indexer}.rs`
- Test: probe `scripts/parity/probes/rust-imports/`, testes de `module_index.rs` e `rust_crates.rs`

- [ ] **Step 1:** Testes que falham para: `pub use m::T` (reexport), `use m::*`, declaração `mod x;` com e sem `#[path]`, receptor tipado (`let r = Repo::new(); r.save()`), `super::` acima da raiz do crate e `impl Tr for T` com `T` de outro arquivo.
- [ ] **Step 2:** Implementar em `module_index.rs`/`rust_crates.rs`, preservando a regra atual de "módulo existe, item não existe → nome solto". Reexports gravam o alvo qualificado mesmo sem nó, como o CRG.
- [ ] **Step 3:** Invalidação: incluir o hash/mtime dos `Cargo.toml` no `detect_changes` do indexer, para que editar só o manifesto re-resolva as fontes. Teste de integração correspondente.
- [ ] **Step 4:** Trocar a alocação de `Vec` por aresta `Calls` em `resolution::reachable` por iterador/fatiamento emprestado; medir `codegraph build .` release antes/depois neste repo e exigir não-regressão.
- [ ] **Step 5:** Harness: `rust-imports` mantém CALLS/IMPORTS_FROM/IMPLEMENTS em 1.000; este repo mantém IMPORTS_FROM ≥ 0.99 e CALLS ≥ 0.964. Documentar o que ficou (glob em contexto não portado, etc.).
- [ ] **Step 6:** Commit `fix(mcp): close cross-file Rust resolution gaps` (+ commit separado para a otimização, se grande).

### Task 6: TypeScript/JS — sobras do extrator

**Files:**
- Modify: `crates/codegraph-parser/src/languages/javascript.rs`, `crates/codegraph-mcp/src/module_index.rs`
- Create: probe `scripts/parity/probes/typescript-js/`

**Interfaces:**
- Produces: nós e chamadas em TSX/JSX, `memo`/`forwardRef`, `require(...)` e `import(...)` como `IMPORTS_FROM`, aliases npm/workspaces, `exports.f = function…`, `Foo.prototype.bar = …` e guarda morta `if (false)`.

- [ ] **Step 1:** Criar o probe com um caso por item; rodar harness e salvar a falha no baseline P1.
- [ ] **Step 2:** Implementar em lotes pequenos (TSX/JSX + wrappers; CommonJS; aliases/workspaces; funções atribuídas a membro; guarda morta), um commit por lote.
- [ ] **Step 3:** Medir: corpus TS/JS permanece 1.000 em nós/CALLS/CONTAINS/IMPORTS_FROM/INHERITS/TESTED_BY; novo probe ≥ 0.95 nas mesmas métricas.
- [ ] **Step 4:** Documentar o que depende de tipo do receptor (`this.users.get → get`) como divergência conhecida.

### Task 7: Go — imports de pacote e testes

**Files:**
- Modify: `crates/codegraph-parser/src/languages/go.rs`, `crates/codegraph-mcp/src/module_index.rs`
- Create: probe `scripts/parity/probes/go-package/`

- [ ] **Step 1:** Probe com módulo multi-pacote (`go.mod` + `replace` + `vendor`) e arquivos `_test.go`; registrar falha atual (IMPORTS_FROM para diretório do pacote e `is_test`).
- [ ] **Step 2:** Resolver o alvo de import como diretório do pacote, não `File`, incluindo `replace`/`vendor`; marcar `is_test=1` em todo nó de `_test.go`; portar o ramo de nome solto ambíguo do CRG que liga ao primeiro símbolo do arquivo.
- [ ] **Step 3:** Harness: corpus Go em 1.000; novo probe ≥ 0.95 em nós/CALLS/IMPORTS_FROM/TESTED_BY.
- [ ] **Step 4:** Commit `fix(parser): resolve Go package imports and test files`.

### Task 8: Java — receptor entre arquivos e anotações de framework

**Files:**
- Modify: `crates/codegraph-parser/src/languages/java.rs`, `crates/codegraph-mcp/src/{resolution,spring}.rs`
- Create: probe `scripts/parity/probes/java-cross-file/`

- [ ] **Step 1:** Probe com tipo importado de outro arquivo, `record`, `@Value`, rota WebFlux e evento Kafka; registrar falha.
- [ ] **Step 2:** Portar `_java_same_package_file` e o receptor tipado com tipo de outro arquivo (inclui `com.x.Cart`); tratar `receiver_form=expression` em testes; ligar gotas de import ao nó de tipo.
- [ ] **Step 3:** Avaliar WebFlux/rotas/Kafka/Temporal/eventos/`@Value` contra o CRG; portar apenas o que o harness mostrar como aresta real, documentar o resto.
- [ ] **Step 4:** Harness: corpus Java 1.000; novo probe ≥ 0.95 em nós/CALLS/IMPORTS_FROM/INHERITS/INJECTS.
- [ ] **Step 5:** Commit `fix(parser): resolve Java cross-file receivers`.

### Task 9: Avaliar o tier `balanced` (LSP)

**Files:**
- Create: `docs/parity/lsp-tier.md`
- Modify: `scripts/parity/README.md`

**Interfaces:**
- Produces: tabela por linguagem (corpus + probes) com `fast` × `balanced` × `full`: Jaccard de nós/CALLS/IMPORTS_FROM, arestas resolvidas e latência de `build`.

- [ ] **Step 1:** Rodar `parity.py corpus` e os probes com `--index-tier balanced` e `--index-tier full`, com os toolchains disponíveis; registrar falhas de LSP por linguagem (não instalar toolchains só para o harness).
- [ ] **Step 2:** Escrever `docs/parity/lsp-tier.md` com casos em que o LSP supera o CRG e casos em que fica atrás, incluindo custo de tempo.
- [ ] **Step 3:** Decidir o padrão. Regra: manter `fast` salvo se `balanced` ganhar ≥ 0.05 em CALLS em pelo menos 3 linguagens e custar ≤ 20% a mais em `build`. Qualquer mudança de padrão vai em commit próprio com atualização de docs e testes.
- [ ] **Step 4:** Commit `docs(parity): evaluate LSP tier against heuristics`.

### Task 10: Desbloquear gramáticas e cobrir as 7 linguagens restantes

**Files:**
- Modify: `crates/codegraph-parser/Cargo.toml`, `crates/codegraph-parser/src/language.rs`, `crates/codegraph-parser/src/languages/{cpp,csharp,ruby,php,swift}.rs`
- Create: `crates/codegraph-parser/src/languages/{kotlin,dart}.rs` se as gramáticas funcionarem; probes em `scripts/parity/probes/` para cada linguagem

**Interfaces:**
- Produces: `IMPORTS_FROM` resolvido (arquivo/módulo/pacote conforme o CRG) e CALLS locais em C#, C/C++, Ruby, PHP, Kotlin, Swift e Dart; `is_test` nos arquivos de teste.

- [ ] **Step 1:** Testar versões de `tree-sitter-kotlin`/`tree-sitter-dart` compatíveis com o `tree-sitter` do workspace. Se nenhuma servir, registrar em `docs/parity/baseline.md` que Kotlin/Dart ficam fora do P1 e escalar decisão — não marcar a tarefa como concluída.
- [ ] **Step 2:** Criar um probe mínimo por linguagem (dois módulos + um teste) e rodar o harness para medir o estado atual.
- [ ] **Step 3:** Implementar por linguagem, em commits separados `feat(parser): resolve imports in <linguagem>`; ordem sugerida: C#, Ruby, PHP, Swift, C/C++, Kotlin, Dart.
- [ ] **Step 4:** Critério por linguagem: nós ≥ 0.95, IMPORTS_FROM ≥ 0.90, CALLS ≥ 0.80 no probe; divergências (ex.: HCL, AOP, eventos, aliases) documentadas, não prometidas.
- [ ] **Step 5:** Rodar `check_p1.py`: nenhuma das 5 linguagens prioritárias pode ter caído.

### Task 11: Fase 3 — decisões de semântica e performance do `update`

**Files:**
- Modify: `crates/codegraph-graph/src/graph_query/{impact,changes}.rs`, `crates/codegraph-mcp-tools/src/review.rs`, `crates/codegraph-mcp-server/src/bin/codegraph.rs`
- Modify: `docs/parity/changes.md`

- [ ] **Step 1:** Comparar com o CRG e decidir (registrar a decisão com fonte em `docs/parity/changes.md`): (a) `tests` do `get_impact_radius` dando um salto de `TESTED_BY` além da profundidade, (b) `affected_files` incluindo os próprios arquivos alterados, (c) entry points Python 284 contra 167.
- [ ] **Step 2:** Implementar as decisões em `impact.rs`/`review.rs` com testes; alvo para entry points: diferença ≤ 5% ou regra explicitamente documentada como intencional.
- [ ] **Step 3:** Perfilhar `codegraph update` no cenário de 2 arquivos (release e debug, 3 repetições, mediana). Alvo: release ≤ 1,2× o CRG (≈18 s) e debug ≤ 2× o release. Se o pós-processo dominar, aplicar `--skip-flows` no caminho incremental e/ou usar os flows incrementais da Task 12.
- [ ] **Step 4:** Rodar os 3 cenários de `docs/parity/changes.md`; arquivos e funções alterados continuam em 1.000 e `impacted` não cai abaixo de 0.948/0.995/0.960.
- [ ] **Step 5:** Commit `fix(review): settle impact semantics and speed up incremental update`.

### Task 12: Flows com ID estável e pós-processo incremental

**Files:**
- Modify: `crates/codegraph-graph/src/analysis/flows/{mod,entry}.rs`, `crates/codegraph-mcp/src/writer.rs`
- Test: `crates/codegraph-graph/tests/analysis_flows_test.rs`

**Interfaces:**
- Produces: `flows.id` derivado de forma determinística do entry-point + membros ordenados (não `last_insert_rowid` de `flows/mod.rs:175`), e recomputação só dos flows cujos membros mudaram.

- [ ] **Step 1:** Teste que falha: dois `finalize` seguidos sem mudança de fonte produzem os mesmos IDs de flow.
- [ ] **Step 2:** Implementar o ID determinístico e a reconciliação incremental mantendo nomes e criticalidades do CRG; `flows.id` antigo pode ser reescrito no primeiro rebuild após mudança de `GRAPH_SHAPE_VERSION`, nunca a cada build.
- [ ] **Step 3:** Harness: entry points, membros e ARI em `docs/parity/baseline.md` não podem cair; medir `flows_s` antes/depois neste repo.
- [ ] **Step 4:** Commit `fix(analysis): make flow IDs stable and post-processing incremental`.

### Task 13: `import_context`, fallbacks de risco e preservação de `extra`

**Files:**
- Modify: `crates/codegraph-parser/src/languages/*`, `crates/codegraph-graph/src/analysis/{insights,risk}.rs`, `crates/codegraph-graph/src/sqlite_store/write.rs:178`
- Test: `crates/codegraph-graph/tests/analysis_insights_test.rs`, `analysis_communities_test.rs`, `sqlite_write_test.rs`

- [ ] **Step 1:** Portar `edges.extra.import_context` dos extratores; em `import_cycles`, filtrar imports type-only, pacote-pai e deferred e excluir nós `extra.virtual`, como o CRG. Comparar o número de pares `File → File` em ciclos neste repo (271 pares hoje) com o CRG.
- [ ] **Step 2:** Portar os três fallbacks de risco — TESTED_BY por nome solto, isenção de arquivos de migration e `0.05·flow_count` — **medindo antes e depois**: `docs/parity/changes.md` mostra 31/35 funções com risco igual; alvo ≥ 34/35 sem piorar o `risk_score` geral.
- [ ] **Step 3:** Em `write.rs`, mesclar o JSON de `extra` em vez de `extra = excluded.extra`, preservando chaves escritas pelo CRG em bancos compartilhados. Teste com banco fixture contendo `extra` do CRG.
- [ ] **Step 4:** Commits separados por item: `fix(analysis): port import context filters`, `fix(analysis): port risk fallbacks`, `fix(storage): merge node extra instead of overwriting`.

### Task 14: Higiene, escala e herança no `AnalysisGraph`

**Files:**
- Modify: `crates/codegraph-graph/src/analysis/insights.rs` (676 linhas), `crates/codegraph-graph/src/graph_query/mod.rs:376`, `crates/codegraph-graph/src/analysis/mod.rs`
- Create: módulos `analysis/insights/{hubs,bridges,gaps,cycles,surprising,questions}.rs`; `docs/parity/analysis-heritage.md`, `docs/parity/scale.md`

- [ ] **Step 1:** Dividir `insights.rs` mantendo a API pública (`hub_nodes`, `bridge_nodes`, `knowledge_gaps`, `import_cycles`, `surprising_connections`, `suggested_questions`) e os testes atuais verdes.
- [ ] **Step 2:** Remover ou migrar `GraphQuery::get_hub_nodes(min_degree)` (hoje usado só em teste); conferir que `graph_tool_executor.rs:1205` continua usando a implementação de `AnalysisGraph`.
- [ ] **Step 3:** Gerar repo sintético com 50k+ nós, medir tempo de build/pós-processo e pico de memória (release); registrar em `docs/parity/scale.md`. Só otimizar se o crescimento for pior que linear.
- [ ] **Step 4:** Herança: medir, com o harness de `scripts/parity/changes/`, o efeito de alimentar `INHERITS`/`IMPLEMENTS` no `AnalysisGraph` (flows, comunidades, `risk_scores`, `review_priorities`, pontes). Se melhorar paridade sem quebrar determinismo, implementar atrás de flag interna; se não, documentar a divisão `Snapshot` vs `AnalysisGraph` como decisão final. Decidir também `OVERRIDES`: portar ou descartar com justificativa.
- [ ] **Step 5:** Commits separados para a divisão, a remoção do método legado e a decisão de herança.

### Task 15: Fechamento do P1

**Files:**
- Modify: `PLANO.md`, `docs/parity/baseline.md`, `docs/parity/changes.md`, `scripts/parity/README.md`, `CHANGELOG.md`

- [ ] **Step 1:** Rodar `python3 scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1-final` e `python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-final`.
- [ ] **Step 2:** `cargo fmt --all -- --check`; `cargo test -p codegraph-parser -p codegraph-mcp -p codegraph-graph`; `cargo test -p codegraph-mcp --test e2e_mcp_tools`; `cargo clippy -p codegraph-parser -p codegraph-mcp -p codegraph-graph --all-targets -- -D warnings` (reportar avisos pré-existentes).
- [ ] **Step 3:** Atualizar `PLANO.md` com os novos números, marcar os itens P1 entregues e mover o que ficou de fora para "Adiado" com motivo.
- [ ] **Step 4:** Commit `docs: close P1 parity and quality plan`.

## Critério de aceite do P1

| Métrica | Linha de base | Alvo |
|---|---|---|
| 5 linguagens prioritárias | nós 0.95–1.00, CALLS 0.923–1.00, IMPORTS_FROM 1.00 | nenhuma queda; python CALLS ≥ 0.95 |
| 7 linguagens restantes | sem resolução de imports | probe por linguagem: nós ≥ 0.95, IMPORTS_FROM ≥ 0.90, CALLS ≥ 0.80 (Kotlin/Dart condicionados à gramática) |
| Caminhos `.` / absoluto / `./../fx/.` | IDs iguais, `qualified_name` cru | `qualified_name` idêntico nos três |
| Cenários de mudança (`changes.md`) | impacted 0.948 / 0.995 / 0.960, risco 31/35 | sem queda; risco ≥ 34/35; entry points Δ ≤ 5% ou decisão documentada |
| `update` 2 arquivos | 70 s debug, 18 s CRG | release ≤ 1,2× CRG; debug ≤ 2× release |
| Flows | ID muda a cada build | ID estável sem mudança; entry points/membros/ARI sem queda |
| Escala | não medida | 50k nós medidos e registro publicado |
| Ferramentas MCP | 27 esquemas + `build_or_update_graph` | inventário comparado com as 31 do CRG, ausências documentadas |

Se um alvo não for atingido, a tarefa não é marcada como ✅: registra-se a medição, a decisão e o motivo em `docs/parity/baseline.md`.
