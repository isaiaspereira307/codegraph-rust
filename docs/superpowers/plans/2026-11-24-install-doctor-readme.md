# Instalação, `uninstall`, `doctor` e README Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `codegraph init` instala skills (e MCP opcional), `codegraph uninstall` desfaz só o que é do CodeGraph, `codegraph doctor` diagnostica sem escrever, e o README descreve o produto atual.

**Architecture:** Tudo por projeto, em cima de `project_init.rs`/`agent_hooks.rs` (crate `codegraph-mcp-server`). Removedores são funções puras texto→texto (testáveis sem disco) que um `uninstall.rs` aplica depois de validar todos os arquivos, no mesmo padrão validate-then-write do `init`. `doctor.rs` é só leitura.

**Tech Stack:** Rust, clap, serde_json, toml, rusqlite (leitura), testes de integração com `CARGO_BIN_EXE_codegraph`.

**Spec:** `docs/superpowers/specs/2026-11-24-install-doctor-readme-design.md`

## Global Constraints

- Só Claude Code e Codex. Nada em `~/.claude`/`~/.codex` é escrito; só `<projeto>/…` (`project_directory` já recusa o home).
- `init` não registra MCP sem `--mcp`; `--mcp` com `--hooks none` é erro.
- Sem manifesto de instalação: `uninstall` só remove o reconhecível (bloco `<!-- codegraph:begin/end -->`, handlers com `command == HOOK_COMMAND`, skills com `.codegraph-managed`, entrada `codegraph` do MCP, bloco do hook de git). Arquivo só é apagado se sobrar vazio.
- `uninstall` padrão `--hooks both`; `--purge-index` é o único jeito de apagar `.code-review-graph/`; `--dry-run` não escreve nada.
- Hooks inline no `.codex/config.toml` (reaproveitados pelo `init`) não são do `init`: `uninstall` nunca reescreve esse TOML por causa deles.
- `doctor` é read-only e sem rede; exit 0 sem `fail`, 1 com `fail`.
- Novos `.rs` começam com duas linhas `// ABOUTME:`. `cargo` sempre com `-p`/`--workspace`. Commits terminam com `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Comandos que não precisam de runtime/config entram nos ramos "handled before configuration" de `bin/codegraph.rs:543-692`.

## Review Focus

- Arquivo do usuário com conteúdo próprio ao lado do bloco do CodeGraph (AGENTS.md, `.mcp.json` com outros servidores, `settings.json` com `permissions`): nada além do nosso muda.
- `uninstall` duas vezes e em projeto nunca inicializado: segunda execução vira no-op com exit 0.
- Symlink em `.claude`, `.mcp.json` etc.: recusado (reuse `reject_symlink`), sem escrita parcial.
- JSON/TOML inválido em um arquivo: erro antes de escrever em qualquer outro.
- `doctor` em diretório sem `.code-review-graph`, com `graph.db` schema 9 e com banco corrompido: não entra em pânico, mostra `fail`/`warn` com a correção.
- `CRLF` em `AGENTS.md`: remoção do bloco preserva o fim de linha.

---

### Task 1: `uninstall` para hooks e instruções

**Files:**
- Create: `crates/codegraph-mcp-server/src/uninstall.rs`
- Modify: `crates/codegraph-mcp-server/src/agent_hooks.rs` (adicionar `strip_settings`), `crates/codegraph-mcp-server/src/project_init.rs` (adicionar `strip_guidance`), `crates/codegraph-mcp-server/src/lib.rs` (declarar o módulo), `crates/codegraph-mcp-server/src/bin/codegraph.rs` (`Commands::Uninstall` tratado antes da config)
- Test: `crates/codegraph-mcp-server/tests/uninstall_integration.rs`; unit tests nos dois módulos

**Interfaces:**
- Produces:
  - `pub(crate) fn strip_settings(settings: Value) -> Result<Value>` (`agent_hooks.rs`): remove handlers cujo `command == HOOK_COMMAND` de `SessionStart`/`SubagentStart`, poda grupos, eventos e o objeto `hooks` vazios; erra como `merge_settings` em formato inválido.
  - `pub(crate) fn strip_guidance(original: &str) -> Result<String>` (`project_init.rs`): remove o bloco marcado (e só ele), preserva `\r\n`, mesma validação de marcadores duplicados/invertidos de `merge_guidance`.
  - `pub(crate) enum FileEdit { Write(Vec<u8>), Delete }` e `pub fn run(project: &Path, hooks: Option<HookSelection>, purge_index: bool, dry_run: bool) -> Result<Vec<(PathBuf, FileEdit)>>` em `uninstall.rs`; `hooks = None` significa `Both`.
  - CLI: `codegraph uninstall [path] [--hooks claude|codex|both] [--purge-index] [--dry-run]`.

- [ ] **Step 1: Testes unitários que falham.** `strip_settings`: round-trip `merge_settings` → `strip_settings` devolve o JSON original (`{}` e um com `permissions`); mantém handler de outro comando no mesmo evento. `strip_guidance`: round-trip com `merge_guidance` para arquivo vazio, com texto antes/depois, com `# codegraph` fora do bloco e com CRLF.
- [ ] **Step 2: Rodar** `cargo test -p codegraph-mcp-server --lib strip_` → FAIL (funções inexistentes).
- [ ] **Step 3: Implementar `strip_settings` e `strip_guidance`**, reaproveitando `visible_lines`/`reject_symlink`/validação existentes.
- [ ] **Step 4: Teste de integração que falha** (`uninstall_integration.rs`, helper `run` copiado de `project_init_integration.rs`): para cada `--hooks claude|codex|both`, `init` com `AGENTS.md` do usuário e `.claude/settings.json` com `permissions`, depois `uninstall`: `AGENTS.md` volta byte a byte ao original, `settings.json` mantém `permissions` sem `hooks`, `.codex/hooks.json` some se ficou `{}`; segunda execução sai com 0 e não altera nada; `--dry-run` não altera nada e lista os arquivos em stdout; JSON inválido em `.codex/hooks.json` falha sem tocar em `.claude/settings.json`.
- [ ] **Step 5: Implementar `uninstall::run` e o subcomando.** Ler e validar todos os alvos primeiro (guia `AGENTS.md`/`CLAUDE.md`, `settings.json`, `hooks.json`, bloco do hook de git via `strip_git_block`, que já existe), depois aplicar. Apagar um arquivo só se o resultado for `{}` ou só espaços. Com `--purge-index`, remover `.code-review-graph/` por último.
- [ ] **Step 6: Rodar** `cargo test -p codegraph-mcp-server --lib --test uninstall_integration --test project_init_integration` → PASS.
- [ ] **Step 7: Commit** (`feat(cli): codegraph uninstall for hooks and instruction blocks`).

### Task 2: Skills de revisão no `init` e no `uninstall`

**Files:**
- Create: `crates/codegraph-mcp-server/src/skills.rs`
- Modify: `project_init.rs` (passo de skills quando a seleção inclui Claude), `uninstall.rs` (remover), `lib.rs`
- Test: `tests/project_init_integration.rs` e `tests/uninstall_integration.rs`

**Interfaces:**
- Consumes: `FileEdit` (Task 1).
- Produces: `pub(crate) const SKILLS: [(&str, &str); 3]` (nome, conteúdo via `include_str!("../../../rules-for-claude-code/skills/<nome>/SKILL.md")`); `pub(crate) fn plan_install(project: &Path) -> Result<Vec<(PathBuf, FileEdit)>>`; `pub(crate) fn plan_uninstall(project: &Path) -> Result<Vec<(PathBuf, FileEdit)>>`. Marcador `.codegraph-managed` dentro de `.claude/skills/<nome>/`.

- [ ] **Step 1: Testes que falham.** `init --hooks claude`: existem os 3 `SKILL.md` e o marcador; `--hooks codex` não cria `.claude/skills`; skill de mesmo nome já existente sem marcador não é sobrescrita (aviso em stderr, exit 0); `uninstall` remove os diretórios com marcador e preserva a skill do usuário.
- [ ] **Step 2: Rodar e ver falhar.**
- [ ] **Step 3: Implementar `skills.rs`** e ligar em `prepare` (validação antes de escrever, como `guidance_changes`) e em `uninstall::run`.
- [ ] **Step 4: Rodar** `cargo test -p codegraph-mcp-server --test project_init_integration --test uninstall_integration` → PASS.
- [ ] **Step 5: Commit** (`feat(init): install review skills for Claude Code`).

### Task 3: `init --mcp` e remoção da entrada MCP

**Files:**
- Create: `crates/codegraph-mcp-server/src/mcp_config.rs`
- Modify: `bin/codegraph.rs` (flag `--mcp` em `Init`), `project_init.rs` (`prepare` ganha `mcp: bool`), `uninstall.rs`
- Test: `tests/project_init_integration.rs`, `tests/uninstall_integration.rs`

**Interfaces:**
- Consumes: `FileEdit`, `HookSelection`.
- Produces: `pub(crate) fn merge_claude_mcp(original: &str) -> Result<String>` (`.mcp.json`: `mcpServers.codegraph = {"command":"codegraph","args":["start","stdio"]}`); `pub(crate) fn strip_claude_mcp(original: &str) -> Result<String>`; `pub(crate) fn merge_codex_mcp(original: &str) -> Result<String>` e `strip_codex_mcp` (`.codex/config.toml`, tabela `[mcp_servers.codegraph]`, mesmos `command`/`args`); `pub(crate) fn plan_install(project, harness) -> Result<Vec<(PathBuf, FileEdit)>>` e `plan_uninstall`.

- [ ] **Step 1: Confirmar o formato do Codex.** Verificar na documentação do Codex se `.codex/config.toml` do projeto lê `[mcp_servers.*]`. Se não ler, `merge_codex_mcp` não é gravado: `init --mcp --hooks codex` imprime o trecho TOML em stderr para colar em `~/.codex/config.toml` e o teste correspondente afirma essa saída. Registrar a conclusão no commit.
- [ ] **Step 2: Testes que falham.** Round-trip merge→strip devolve o original (arquivo com outro servidor MCP preservado, JSON/TOML sem a chave, arquivo ausente); `init --hooks claude --mcp` cria `.mcp.json` com a entrada e sem tocar nas outras; `init --hooks none --mcp` sai com código ≠ 0 e mensagem citando `--hooks`; sem `--mcp` nenhum arquivo MCP é criado; `uninstall` remove só a entrada e apaga `.mcp.json` se ficou `{"mcpServers":{}}` sem mais nada.
- [ ] **Step 3: Rodar e ver falhar.**
- [ ] **Step 4: Implementar** `mcp_config.rs` e ligar em `prepare` e `uninstall::run`; recusar symlink e conteúdo inválido antes de qualquer escrita.
- [ ] **Step 5: Rodar** `cargo test -p codegraph-mcp-server --lib --test project_init_integration --test uninstall_integration` → PASS.
- [ ] **Step 6: Commit** (`feat(init): optional --mcp registration for Claude Code and Codex`).

### Task 4: `codegraph doctor`

**Files:**
- Create: `crates/codegraph-mcp-server/src/doctor.rs`
- Modify: `lib.rs`, `bin/codegraph.rs` (`Commands::Doctor { path, json }`, antes da config), `Cargo.toml` do crate se `rusqlite` não for dependência (usar a do workspace)
- Test: `crates/codegraph-mcp-server/tests/doctor_integration.rs`; unit tests de cada verificação em `doctor.rs`

**Interfaces:**
- Produces: `pub enum Status { Ok, Warn, Fail }`; `pub struct Check { pub name: &'static str, pub status: Status, pub detail: String, pub fix: Option<String> }`; `pub fn run_checks(project: &Path) -> Vec<Check>`; `pub fn render_table(checks: &[Check]) -> String`; `pub fn render_json(checks: &[Check]) -> String` (`{"checks":[{"name","status","detail","fix"}]}`, `status` em minúsculas). Verificações, nesta ordem e com estes `name`: `binary`, `crg`, `graph_db`, `hooks`, `git_hook`, `skills`, `mcp`, `write_lock`.

- [ ] **Step 1: Testes que falham** (integração, projeto temporário): sem `.code-review-graph` → `graph_db` `fail` com fix `codegraph build .`; com banco criado por `codegraph build` → `graph_db` `ok` e detalhe com contagem de nós; banco com `schema_version = 9` (criar com rusqlite) → `fail` citando a versão; nós `File` com `@L` no nome (inserir à mão) → `warn` mandando rodar `codegraph build`; `init --hooks both` → `hooks` `ok`, antes do `init` → `warn`; `--json` é JSON válido com as 8 chaves; exit code 1 com algum `fail`, 0 sem.
- [ ] **Step 2: Rodar** `cargo test -p codegraph-mcp-server --test doctor_integration` → FAIL.
- [ ] **Step 3: Implementar as verificações.** `graph_db` abre com `SQLITE_OPEN_READ_ONLY` (nunca por `SqliteGraphStore::open`, que cria/migra). `crg`: roda `code-review-graph --version`; ausente = `ok` "não instalado"; versão < 2.4.0 + comando contendo `code-review-graph` em `~/.claude/settings.json` + ausência de `.code-review-graph/crg-hooks-off` → `warn` (fix: criar o marcador). `hooks`: reutiliza o reconhecimento por `HOOK_COMMAND` (`configured_event`, tornar `pub(crate)`) e confere `codegraph` no PATH. `binary`: caminho do `codegraph` no PATH contra `std::env::current_exe()`, `warn` se diferirem. `write_lock`: tenta `flock` não bloqueante só para leitura do estado; informativo.
- [ ] **Step 4: Rodar** `cargo test -p codegraph-mcp-server --test doctor_integration --lib doctor` → PASS. Rodar também `codegraph doctor .` neste repo e ler a saída.
- [ ] **Step 5: Commit** (`feat(cli): codegraph doctor`).

### Task 5: README e documentos legados

**Files:**
- Modify: `README.md` (reescrita, ~250 linhas), `docs/README.md`, `install-codegraph*.sh` (banners), `CLAUDE.md` (seção "Stale references to ignore"), `PLANO.md` (marcar o item `install`/`doctor` da Fase 5 e o README como feitos)
- Test: verificação por script (abaixo)

- [ ] **Step 1: Reescrever o `README.md`** com as seções: o que é; instalação (`cargo install --path crates/codegraph-mcp-server --bin codegraph --all-features --force`); início rápido (`init`, `build`, `tool run`); ferramentas (colar a saída de `codegraph tool list`, só nomes e uma linha); MCP opcional (`init --mcp` e configuração manual para Claude Code e Codex); `doctor` e `uninstall`; convivência com o CRG (schema v15, hooks globais do CRG 2.3.x, marcador `crg-hooks-off`, link para `docs/architecture/storage-sqlite.md`); linguagens (link `docs/SUPPORTED_LANGUAGES.md`); tiers; licença (`LICENSE-MIT`/`LICENSE-APACHE`). Nenhuma menção a SurrealDB, embeddings, LLM ou ferramentas agênticas.
- [ ] **Step 2: Corrigir banners dos `install-codegraph*.sh` e `docs/README.md`** onde descrevem o produto antigo (texto de banner/índice apenas, sem mudar a lógica dos scripts). Remover do `CLAUDE.md` as linhas "stale" que deixaram de ser verdade.
- [ ] **Step 3: Verificar.** `grep -inE 'surreal|embedding|openai|ollama|jina|agentic' README.md` não imprime nada; todo link relativo do README existe (`grep -oE '\]\(([^)#h][^)]*)\)' README.md | tr -d '()]' | sed 's/^\[//' | while read f; do test -e "$f" || echo "missing $f"; done` não imprime nada); cada comando mostrado no início rápido roda num projeto temporário (`codegraph init . --hooks claude --no-index`, `codegraph uninstall .`, `codegraph doctor .`).
- [ ] **Step 4: Commit** (`docs: rewrite README for the SQLite/no-LLM product, fix legacy banners`). Depois `cargo clean` (memória do projeto).

---

## Self-review

- Cobertura do spec: `init` skills (T2), `--mcp` (T3), `uninstall` (T1-T3), `doctor` com as 8 verificações (T4), README e legados (T5). A decisão "sem manifesto" está nas Global Constraints e testada em T1.
- Consistência de tipos: `FileEdit` nasce em T1 e é consumido em T2/T3; `plan_install`/`plan_uninstall` têm a mesma forma em `skills.rs` e `mcp_config.rs`.
- Risco aberto: T3 Step 1 depende de a documentação do Codex confirmar a leitura de `.codex/config.toml` por projeto; o plano já traz o caminho alternativo.
