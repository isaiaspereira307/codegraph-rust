# Plano: code-review-graph em Rust

Objetivo: transformar este fork do `codegraph-rust` numa versão em Rust do
[code-review-graph](../code-review-graph) (CRG): um binário único, sem Python e sem LLM, que
indexa o repositório num grafo local e entrega a agentes de IA (via MCP e CLI) contexto de
revisão determinístico e barato em tokens.

Referência de comportamento: `../code-review-graph` (Python, v2.4.0, ~91k linhas, 31
ferramentas MCP, schema SQLite versão 15).

---

## Fase 0 — Limpeza da camada de IA ✅ (commit `533a821` na `main`)

Feito:

- Removidos os crates `codegraph-mcp-rig` (agente LLM), `codegraph-vector` (embeddings,
  reranking), `codegraph-zerocopy` e `codegraph-concurrent`. Workspace: 12 → 8 crates.
- Removidas as features `ai-enhanced`, `embeddings-*`, `cloud`, `rig-*` e todo o código
  `#[cfg]` correspondente (indexer, resolução semântica, servidor, testes).
- Servidor MCP reescrito (`official_server.rs`): as 4 ferramentas "agentic" deram lugar a 8
  ferramentas de grafo diretas, com parâmetros tipados. Nova ferramenta `find_nodes_by_name`
  para obter `node_id` sem busca semântica.
- `codegraph agent …` → `codegraph tool run|list|instructions` (`tool_cli.rs`).
- Hooks do Claude/Codex, guias de instrução (`src/prompts/*.md`), `CLAUDE.md`, `AGENTS.md`,
  Makefile e workflow de release atualizados. Scripts Python de avaliação agentic removidos.
- −5,1k linhas nos arquivos alterados, além dos crates removidos.

Pendente para fechar a fase:

- [x] `cargo test -p codegraph-mcp --test e2e_mcp_tools` verde (5/5). Suíte completa
      `cargo test --workspace --all-features --no-fail-fast` verde, pulando os 5 testes de
      watcher do WSL (ver notas de ambiente); `cargo check --workspace --all-targets
      --all-features` limpo.
- [x] Commit e push direto na `main` do fork (`isaiaspereira307/codegraph-rust`).
      Fluxo do projeto: sem PR e sem interação com o upstream.

Sobras conhecidas (config/embedding e `graph_vector.rs` removidos na Fase 1, Task 7):

- `README.md`, `docs/` e banners dos `install-codegraph*.sh` ainda descrevem o produto antigo.

### Notas de ambiente (máquina local, WSL)

- **OpenSSL**: resolvido na Fase 1 (Task 7). O workspace não depende mais de `openssl-sys`
  nem `native-tls`; o binário release não linka `libssl`, então `LD_LIBRARY_PATH` não é mais
  necessário.
- **Watchers no WSL**: 2 testes de `codegraph-core::watch` falham também no código original.
  3 testes de watcher de `codegraph-parser` travam (`test_file_change_detection`,
  `test_file_monitoring_only`, `test_integration_test_suite`). Rodar a suíte com
  `--skip` nesses testes até investigar (provável limitação de inotify em WSL).
- **Clippy estrito** (`-D warnings`) falha em diagnósticos que já existiam em `codegraph-core`.

---

## Fase 1 — Storage: SurrealDB → SQLite + petgraph ✅

Plano detalhado: `docs/superpowers/plans/2026-11-19-fase1-storage-sqlite.md`.

Progresso:

- [x] Task 1 — schema CRG v15 e abertura do banco (`21b3f30`).
- [x] Task 2 — escrita de nós, arestas e metadados (`21aa11f`).
- [x] Task 3 — reconciliação, FTS e lock de escrita (`ea9de2f`).
- [x] Task 4 — camada de consulta petgraph para as 8 ferramentas (`b9e56b9`).
- [x] Task 5 — executor, servidor MCP, `tool run` e `db-check` lendo do SQLite.
- [x] Task 6 — writer e indexer gravando no SQLite (`codegraph index` cria
      `.code-review-graph/graph.db`; lock de escrita e `finalize` por execução).
- [x] Task 7 — remover SurrealDB, `schema/*.surql`, `graph_vector.rs`, `reqwest` e
      `native-tls`; binário sem `libssl` (`ldd` sem ssl). O cliente WebSocket legado de
      `codegraph-mcp` aceita só `ws://`.
- [x] Sobras de embedding/LLM/rerank: `EmbeddingConfig`/`LLMConfig`/`RerankConfig`,
      `advanced_config`/`cli_config`/`embedding_config`, `context_aware_limits`, flags de
      embedding do `index`, ETA do `estimate` (fica contagem + tempo de parse), templates
      `.env`/TOML. `--batch-size` agora é o lote de escrita no SQLite. Seções antigas no TOML
      são ignoradas; de `[llm]` só `context_window` é lido. Ficam no indexer os estágios de
      inferência sempre desligados (`policy.rs`, campos `embeddings`/`inference_*` das stats).

Por quê: compatibilidade com o banco do CRG, concorrência (SurrealKV trava o diretório para
um único processo, e o CRG roda watch + hooks + MCP ao mesmo tempo), licença (SurrealDB é
BSL), build menor (sem C++/OpenSSL) e testes de paridade diretos entre os dois bancos.

Tarefas:

1. Criar o crate `codegraph-store` (ou reescrever `codegraph-graph`) com `rusqlite`
   (`bundled`), WAL, e o schema do CRG versão 15 (`graph.py:_SCHEMA_SQL` +
   `migrations.py`): `nodes`, `edges`, `metadata`, `flows`, `flow_memberships`,
   `flow_snapshots`, `communities`, `community_summaries`, `risk_index`, FTS de nós
   (`nodes_fts`). Banco em `.code-review-graph/graph.db`, para ler índices existentes.
2. Mapear tipos: `codegraph_core::CodeNode`/`EdgeType` ↔ `nodes.kind` (File, Class, Function,
   Test, …) e `edges.kind` do CRG (CALLS, IMPORTS_FROM, INHERITS, CONTAINS, TESTED_BY, …).
   Documentar as perdas em cada direção.
3. Trocar o `writer.rs` e o `indexer.rs` do `codegraph-mcp` para gravar no SQLite;
   manter a reconciliação incremental (hash por arquivo, remoção de registros obsoletos).
4. Camada de consulta em memória com `petgraph`, carregada sob demanda e invalidada por
   `metadata.last_updated`: dependências transitivas e reversas, ciclos (Tarjan), hubs,
   coupling, call chain. Reimplementar as 8 ferramentas atuais sobre ela, no lugar das
   funções `fn::*` em SurrealQL.
5. Remover `surrealdb`, `schema/*.surql`, `SurrealDbStorage`, `CODEGRAPH_SURREALDB_*`
   e as sobras de embedding listadas na Fase 0 (`db-check` ficou, agora sobre o SQLite).
6. Migrar o lock de processo único para o modelo do CRG (vários leitores, um escritor; WAL).

Critério de aceite: `codegraph index` + `codegraph tool run …` + `codegraph start stdio`
funcionando sem SurrealDB; o binário não depende de `libssl`; as 8 ferramentas com testes
sobre um repositório fixture.

---

## Fase 2 — Paridade do parser

Risco principal do projeto: o `parser.py` do CRG tem ~25k linhas de regras por linguagem,
mais resolvers específicos (C#, AOP, eventos, HCL, aliases npm, workspaces, jedi), contra
~12k linhas no `codegraph-parser`.

Plano detalhado (harness + camada comum): `docs/superpowers/plans/2026-11-20-fase2-paridade-parser.md`.

Estado antes do plano 1 (histórico; piloto de 2026-11-20, 7 fixtures de
`../code-review-graph/tests/fixtures`): CRG 90 nós / 203 arestas, Rust 96 / 29. A maior
diferença não era por linguagem, era de forma do grafo: o Rust não gerava nó `File`,
`CONTAINS`, `Test`/`TESTED_BY`; gravava métodos sem a classe no nome (`__init__@L10`), imports
como nós `Import` com `IMPORTS_FROM` em laço e descartava chamadas não resolvidas (o CRG guarda
o texto do alvo). Tudo isso foi corrigido pela camada comum (`crg_shape`); ver "Progresso da
Fase 2" abaixo. Ainda aberto: `INHERITS` (e `IMPLEMENTS`/`REFERENCES`) não são gerados.

Atenção: o `code-review-graph` instalado no PATH é 2.3.7 e cria schema **9**; o hook de
SessionStart dele já gravou `.code-review-graph/graph.db` v9 neste repo, que o Rust recusa
abrir. O harness usa o CRG da fonte (`uv run --frozen --project ../code-review-graph`, v2.4.0,
schema 15). Atualizar o CRG instalado ou desligar o hook dele neste repo.

1. Harness de paridade (`scripts/parity/`): indexar o mesmo repositório com o CRG (Python) e
   com o binário Rust; comparar os dois `graph.db` por `(kind, qualified_name, file, linhas)`
   para nós e `(kind, source, target)` para arestas. Relatório por linguagem: só-no-CRG,
   só-no-Rust, divergentes.
2. Corpus de teste: repositórios pequenos e fixos por linguagem (começar por Python, TS/JS,
   Go, Rust, Java; depois C#, C/C++, Ruby, PHP, Kotlin, Swift, Dart), mais os fixtures de
   `../code-review-graph/tests`.
3. Fechar as diferenças linguagem a linguagem, priorizando arestas CALLS/IMPORTS e a
   detecção de testes (`is_test`, TESTED_BY), que alimentam o raio de impacto.
4. Avaliar o tier `balanced` (LSP) como substituto dos resolvers heurísticos; registrar em
   que casos o LSP supera o CRG e em quais fica atrás.

Critério de aceite: ≥95% de concordância de nós e ≥90% de arestas CALLS/IMPORTS nas
linguagens prioritárias, com as divergências restantes documentadas.

### Progresso da Fase 2

Plano 1 (harness + camada comum) concluído, commits `d5f4e09..HEAD`: `scripts/parity/`
(corpus de 5 linguagens, `parity.py run|corpus`), nós `File`/`CONTAINS`, nomes qualificados
por classe/dono (`crg_shape`), imports como `IMPORTS_FROM File → módulo` resolvidos por
linguagem como o CRG (senão ficam como texto; nós `Import` removidos), nós `Test`
(`is_test`), alvos não resolvidos guardados como texto, `TESTED_BY` derivado no finalize e
nomes soltos ligados só no mesmo arquivo ou via arquivo importado. Linha de base:
`docs/parity/baseline.md` (antes e depois).

Jaccard depois do plano 1 (antes → depois, nós / CALLS / IMPORTS_FROM):

| linguagem | nós | CALLS | IMPORTS_FROM |
|---|---|---|---|
| python | 0.51 → 1.00 | 0.12 → 0.55 → **0.92** | 0 → 1.00 |
| typescript | 0.20 → 0.44 → **1.00** | 0 → 0.13 → **1.00** | 0 → 1.00 |
| go | 0.30 → 1.00 | 0.16 → 0.13 → **1.00** | 0 → 1.00 |
| rust | 0.38 → 0.74 → **1.00** | 0.17 → 0.50 → **1.00** | 0 → 0.50 → **1.00** |
| java | 0.17 → 0.95 → **1.00** | 0 → 0.35 → **1.00** | 0 → 1.00 |

### Próximos planos por linguagem

Ordem pela pior concordância de CALLS/IMPORTS_FROM (TESTED_BY junto). TS e Go empatam em
CALLS (0.133 vs 0.129); TS vai primeiro por também ter os piores nós, CONTAINS e TESTED_BY:

1. ✅ **TypeScript/JS** (plano `docs/superpowers/plans/2026-11-20-fase2-typescript.md`,
   commits `c0ee8f0..HEAD`): métodos, interface/type/enum como `Class`, arrows nomeadas,
   nome final da chamada, chamadas no módulo saem do `File`, nós `Test` de
   `describe`/`it`/`test`, `INHERITS` como texto. Corpus: 1.00 em nós, CALLS, CONTAINS,
   IMPORTS_FROM, INHERITS e TESTED_BY. Fica para depois: `REFERENCES` (comum com Python),
   TSX/JSX e wrappers de componente (`memo`/`forwardRef`), aliases npm e workspaces, guardas
   mortas (`if (false)`), CommonJS (`require(...)`, `import(...)` como IMPORTS_FROM) e funções
   atribuídas a membro (`exports.f = function…`, `Foo.prototype.bar = …`, CRG
   `_extract_js_member_functions`), valores `function*` e campos privados `#x`.
   `INHERITS` fica como texto só para TS/JS (único extrator que gera `Extends` hoje); o
   `GraphQuery` ignora alvos de texto, então as ferramentas não seguem herança até ele
   resolver `INHERITS` por nome.
2. ✅ **Go** (plano `docs/superpowers/plans/2026-11-20-fase2-go.md`): todo `type_spec` vira
   `Class` (alias `type A = B` não vira nó), nome final da chamada, chamador = função nomeada
   (func literals não contam; fora de função sai do `File`), guarda morta `if false`, `recv.M()`
   no receptor não sombreado resolvido para `Tipo.M` (port do `_go_receiver_binding_index`),
   embedding como `INHERITS` texto. Corpus: 1.00 em nós, CALLS, CONTAINS e IMPORTS_FROM.
   Medido no CRG: não há ligação no mesmo pacote entre arquivos (um `helper()` em `a.go` não
   liga ao de `b.go`). Fica para depois: `IMPORTS_FROM` para o diretório do pacote via
   `go.mod`/`replace`/`vendor` (alvo do CRG é um diretório, não um `File`), `is_test=1` em todo
   nó de `_test.go`, e o ramo do CRG que liga nome solto ambíguo ao primeiro símbolo do arquivo.
   A guarda morta só existe nos extratores Go e Rust; TS ainda não a aplica.
3. ✅ **Java** (plano `docs/superpowers/plans/2026-11-20-fase2-java.md`): sem nó `package`
   (vira `java_package`), enum como `Class`, `java_kind`, chamador por pilha (inicializador de
   campo sai do `File`), `new X()`/`X::m` como chamadas, receptor tipado por campo/parâmetro/
   local (`<arquivo>::Tipo.m` quando o tipo é do arquivo), chamada sem receptor ligada ao
   primeiro homônimo do arquivo (`super.save`), `INHERITS` e `INJECTS` (Spring/Lombok) como
   texto com `java_type`, pré-passe Spring no indexer (`spring.rs`, porta `spring_resolver.py`),
   `@Test` por anotação (atributo `decorators` lido pelo `crg_shape`). Corpus: 1.00 em nós,
   CALLS, CONTAINS, IMPORTS_FROM, INHERITS e INJECTS. Fica para depois: receptor tipado com tipo
   de outro arquivo (import resolvido, `_java_same_package_file`, tipo `com.x.Cart`), WebFlux/
   rotas/Kafka/Temporal/eventos/`@Value`, `receiver_form=expression` em testes, `record`.
4. ✅ **Rust** (plano `docs/superpowers/plans/2026-11-20-fase2-rust.md`): `impl`, `mod` e macros
   não viram nó (o `impl` só põe os métodos no tipo, em qualquer profundidade), assinaturas e
   métodos de trait em `Trait.m`, `#[test]`/`#[tokio::test]` como `decorators` → `Test`,
   `IMPLEMENTS` (traço do arquivo resolvido, senão texto), chamador por pilha (const de `impl` sai
   do `File`), nomes de chamada pelo caminho (`HashMap::new`, `Vec::<u8>::new` → `Vec::new`),
   `receiver`, `self.m()`/`Self::m` → `Tipo.m`, `X::m`/`mod::f` resolvidos no arquivo, macros como
   chamadas, guarda morta `if false` (para no `impl`), `use` achatado pela árvore (glob no topo
   não gera import). Caminhos não resolvidos ficam como texto com `crg_verbatim_target`: o indexer
   só os liga por alias exato (sem normalizar nem busca léxica). `enum` vira `Class`. Como o
   `_settle_unbound_endpoints` do CRG (vale para toda linguagem), CALLS/REFERENCES para
   `<arquivo>::Dono.m` sem nó viram `m` (`self.clone()` derivado), sem busca léxica; chamada com
   receptor sem tipo (`x.m()`, não `self`) só liga a função livre, nunca a método. Corpus: 1.00 em nós, CALLS,
   CONTAINS, IMPLEMENTS, IMPORTS_FROM e TESTED_BY, e determinístico. Fica para depois: resolução
   de módulos entre arquivos (`_resolve_rust_module_file`: raiz do crate por `lib.rs`/`main.rs`,
   `crate::`/`super::`/dependências do `Cargo.toml`, `use m::f` com primeiro segmento solto,
   busca limitada à raiz do repositório) e, com ela, `IMPLEMENTS`/`X::m` para itens de outro
   arquivo; `IMPLEMENTS` e CONTAINS de `impl` cujo tipo não está no arquivo (o CRG grava origem
   `F::Tipo` sem nó); receptor tipado (`let r = Repo::new(); r.save()`).
5. ✅ **Python** (plano `docs/superpowers/plans/2026-11-20-fase2-python.md`, commits
   `d637b61..11cbcb5`): extrator reescrito no molde Go/Java/Rust (pré-passe de nomes de topo,
   imports `from`/`import` e `TYPE_CHECKING`; chamador por pilha, corpo de classe e decorators
   saem do `File`), nome de chamada e receptor do CRG (`self`/`cls`/classe local → `Classe.m`;
   nome importado leva `crg_py_import` "módulo|símbolo", resolvido pelo `ModuleIndex`; resto
   `crg_unbound`), guarda morta estática (`if False`, `TYPE_CHECKING`), `INHERITS` texto,
   `REFERENCES` de valores, `is_test` em todo nó de arquivo de teste (todas as linguagens),
   classe aninhada com escopo-folha (cadeia completa quando a folha é ambígua), candidatos de
   submódulo em `from M import x`, `_python_folder_hides_module`, `import a.b` liga `a` e nome
   de módulo solto (`from pkg import b; b()`) liga ao `File`. Corpus: 1.00 em nós (sem
   divergentes), CONTAINS, IMPORTS_FROM, INHERITS e TESTED_BY; CALLS 0.92 (falta só
   `process_request(service: AuthService)` → `AuthService.authenticate`, receptor por
   anotação). Sonda `scripts/parity/probes/python`: 1.00 em nós, CALLS, IMPORTS_FROM, INHERITS e
   TESTED_BY; CONTAINS 0.96. Fica para depois: `REFERENCES` no tier padrão (o `fast` descarta
   `References`; com `--index-tier full` o corpus e a sonda dão 1.00 — decidir se as referências
   de valor no formato do CRG passam a valer no `fast`); CONTAINS de membro de classe aninhada
   (o CRG grava a origem como texto `F::Inner`, sem nó; o writer só grava origem por nó);
   evidência de receptor do CRG (`_apply_python_receiver_evidence`): receptor tipado por
   anotação/atribuição, caminho de módulo pontuado (`import q; q.lib.f()`) e construtor de
   classe de módulo (`mod.Thing().go()`); reexportação de pacote (`_python_reexport_origin`) e
   shims `sys.modules`; reescrita pós-build por sufixo (`python_resolver.py`); `line` das
   arestas; escopo-folha de classe aninhada em C++. A lista da stdlib é a do CPython 3.15; o CRG
   usa `sys.stdlib_module_names` do interpretador que o roda (3.13 no harness: difere em
   `annotationlib`, `compression`, `profiling`, `sre_*` e módulos `_privados`). Dois custos da paridade: (a) o CALLS resolvido do corpus caiu de
   1.000 para 0.833, porque `crg_unbound` impede que receptor tipado por anotação
   (`service.authenticate()`) seja ligado lexicalmente; "quem chama `authenticate`" perde essas
   arestas até o receptor tipado ser portado; (b) os `References` antigos de dica de tipo do
   tier `full` (`parameter_type`/`return_type`) foram removidos.
6. Demais linguagens (C#, C/C++, Ruby, PHP, Kotlin, Swift, Dart): nenhuma resolve imports
   ainda; corpus e fixtures a criar.

Dívida menor registrada: hotspots de complexidade ignoram nós `Test`; atributos
`crg_shaped`/`pre_shape_qualified_name` em todo nó (tamanho).

- `line` das arestas é 0 em CALLS/IMPORTS_FROM, e o `TESTED_BY` herda o 0 (o CRG grava a
  linha da chamada; `source_span` pode preenchê-la).
- Alvos de texto não resolvidos aumentam o banco e a carga do snapshot (igual ao CRG).
- `resolution::reachable` aloca um vetor por alias; medir em repositórios grandes.
- TS: chamada com receptor grava só o nome final (`this.users.get` → `get`) e pode ligar a um
  `get` único no mesmo arquivo/importado; o CRG usa o tipo do receptor para evitar isso.
  `this.x()` com dois métodos `x` no arquivo fica sem resolver (o CRG qualifica pela classe e
  sobe a herança).
- IDs de nó dependem de indexar por `.` ou pelo caminho absoluto (anterior ao plano;
  sufixos `@L` mudam em execuções mistas) — precisa de issue própria.

Polimento do harness (parqueado): `_check_crg_schema` sem tabela `metadata` gera traceback;
`FileNotFoundError` quando faltam `uv`/`git`; incluir stdout quando a ferramenta falha;
`_emit` duplicado em `compare`; URI `file:` sem escape; divergência de linguagem não
reportada.

---

## Fase 3 — Núcleo de revisão (MVP usável)

Ferramentas MCP e comandos CLI equivalentes aos do CRG:

| CRG | Rust | Base existente |
|---|---|---|
| `build_or_update_graph_tool` | `build_or_update_graph` | indexer incremental |
| `detect_changes_tool` | `detect_changes` | `codegraph-parser/src/diff.rs` + `git diff` |
| `get_impact_radius_tool` | `get_impact_radius` | reverse deps (Fase 1) |
| `get_review_context_tool` | `get_review_context` | novo: nós alterados + vizinhos + testes + snippets |
| `get_minimal_context_tool` | `get_minimal_context` | novo: versão enxuta para economia de tokens |
| `query_graph_tool` | `query_graph` | callers/callees/imports/tests_for |
| `traverse_graph_tool` | `traverse_graph` | BFS no petgraph |
| `list_graph_stats_tool` | `list_graph_stats` | contagens no SQLite |
| `semantic_search_nodes_tool` | `search_nodes` | FTS5 (sem embeddings) |
| `find_large_functions_tool` | `find_large_functions` | `find_complexity_hotspots` |

Também: subcomandos `build`, `update`, `status`, `watch`, `detect-changes`, `impact`, `query`,
`search`; hook de `SessionStart` e hook de git pós-commit para update incremental; skills
`review-changes`/`review-pr`/`review-delta` adaptadas.

Critério de aceite: num repositório real, o fluxo "editar → `detect_changes` →
`get_review_context`" devolve o mesmo conjunto de arquivos/funções afetados que o CRG (via
harness da Fase 2) e mede menos tokens que ler os arquivos diretamente.

---

## Fase 4 — Análise de grafo

1. Flows: `list_flows`, `get_flow`, `get_affected_flows` (portar `flows.py`; usar a base de
   call chain).
2. Comunidades: `list_communities`, `get_community`, `get_bridge_nodes`,
   `get_architecture_overview`. Não há crate madura de Leiden: implementar Louvain/Leiden
   sobre `petgraph` com seed fixa (reprodutível) e manter o fallback por arquivo do CRG.
3. Insights: `get_hub_nodes` (já existe), `get_knowledge_gaps`,
   `get_surprising_connections`, `get_suggested_questions`.
4. `risk_index`: portar o cálculo de risco usado no review context.

---

## Fase 5 — Produto e extras

- `refactor` / `apply_refactor` (rename com preview; `refactor.py`).
- Multi-repo: `register`, `unregister`, `repos`, `list_repos`, `cross_repo_search`.
- Wiki: `generate_wiki`, `get_wiki_page`, `get_docs_section` (sem LLM por padrão).
- Visualização HTML (reaproveitar `code_review_graph/assets`), `export`.
- Instalador para as plataformas do CRG (`skills.py:PLATFORMS`): codex, claude, cursor,
  windsurf, zed, continue, opencode, gemini-cli, qwen, kiro, copilot, copilot-cli e outras;
  `install`, `uninstall`, `doctor`.
- Embeddings opcionais (feature desligada por padrão) via ONNX local, se a busca FTS5 se
  mostrar insuficiente.
- Distribuição: release com binários (Linux/macOS/Windows), Homebrew, GitHub Action
  equivalente ao `action.yml` do CRG.
- Reescrever `README.md`/`docs/`, renomear o binário/crates e definir o nome do projeto.

---

## Decisões em aberto

1. ~~**Compatibilidade de banco**~~: decidido na Fase 1 — mesmo schema v15, mesmo
   `.code-review-graph/graph.db` do CRG.
2. **Nome do binário**: manter `codegraph` ou usar `code-review-graph`/`crg`?
3. **Tiers com LSP**: manter os tiers `balanced`/`full` (mais precisos, exigem language
   servers instalados) ou só o `fast` na v1? Recomendação: manter, com `fast` como padrão.
4. **Licença do fork**: o `Cargo.toml` declara "MIT OR Apache-2.0", mas não há arquivo
   `LICENSE`; adicionar um no fork.

## Ordem sugerida e marcos

1. ~~Fechar a Fase 0~~ ✅.
2. ~~Fase 1 (storage)~~ ✅.
3. Harness da Fase 2 antes de qualquer ferramenta nova, para medir regressões desde o início.
4. Fase 3 → **MVP**: binário instalável que substitui o CRG no fluxo de revisão do Claude Code.
5. Fases 4 e 5 conforme o uso real pedir.
