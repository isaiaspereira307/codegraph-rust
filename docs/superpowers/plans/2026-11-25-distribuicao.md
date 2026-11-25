# Distribuição Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Entregar o `codegraph` como binário instalável (release no GitHub para Linux/macOS/Windows, script `curl | sh`, Homebrew) e uma GitHub Action que comenta o risco de PRs, sem Python.

**Architecture:** Um workflow `release.yml` novo compila nativamente em 5 runners, empacota e publica os assets com checksums. Todo o resto consome esses assets: `install.sh`, a fórmula Homebrew e a Action composta. O relatório de PR vira o subcomando Rust `codegraph pr-report` (substitui o `render_pr_comment.py` do CRG).

**Tech Stack:** GitHub Actions, bash, Rust (clap, serde_json), Homebrew (Ruby), `gh` CLI.

**Spec:** `PLANO.md`, seção "Próximos passos", item 3 ("Distribuição"); referência de comportamento da Action: `../code-review-graph/action.yml` e `../code-review-graph/docs/GITHUB_ACTION.md`.

## Global Constraints

- Repositório: `isaiaspereira307/codegraph-rust`; sem PR, push direto na `main`; sem interação com o upstream (`PLANO.md`, Fase 0).
- Binário e crates continuam `codegraph` (Decisão 2); nome do asset: `codegraph-<versão>-<target>.tar.gz` (`.zip` no Windows), com `codegraph` (ou `codegraph.exe`), `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`.
- Targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
  - Nota (2026-11-25, revisão final): Windows foi removido do v1 (não compila: `mmap.rs` usa `windows_sys` incorretamente e `nix` é dependência incondicional). Release e CI cobrem 4 alvos; `package.sh` mantém o suporte a zip.
- Sem `libssl`: o binário release não pode linkar OpenSSL (Fase 1, Task 7); nenhum passo de CI instala `libssl-dev`.
- Licença `MIT OR Apache-2.0`; schema do banco v15 (`.code-review-graph/graph.db`).
- Limiares de risco da Action, como no CRG: `high` ≥ 0.70, `critical` ≥ 0.85.
- Comentário de PR: marcador `<!-- codegraph-report -->` na primeira linha; teto de 12.000 bytes nas seções extras e 60.000 bytes no total (limite do GitHub é 65.536).
- Cada tarefa fecha com `cargo fmt --all` limpo e commit; mensagens terminam com a linha `Co-Authored-By` exigida.
- Criar tag, publicar release e criar o repositório do tap são ações externas: só com confirmação do usuário (Task 8).

## Review Focus

- `install.sh` em OS/arquitetura sem asset (ex.: FreeBSD, `riscv64`): sai com 1 e mensagem listando os targets suportados, sem baixar nada.
- Checksum que não confere: aborta com 1 e não deixa binário parcial no destino.
- Diretório de instalação com espaço no caminho (`$HOME/my bin`): instala normalmente.
- `pr-report` com diff vazio: relatório "nenhuma mudança analisada", exit 0, sem nota de risco inventada.
- `pr-report` com grafo ausente/ilegível ou `detect_changes` falhando: exit ≠ 0 (nunca um comentário tranquilizador sobre um PR não analisado).
- Relatório maior que o limite do GitHub: truncado com nota do que foi cortado, ainda com o marcador na primeira linha.
- PR vindo de fork (token só de leitura): o passo de comentário avisa e a Action segue; o relatório fica na saída `comment-file`.
- Release dry-run (`workflow_dispatch` sem tag): compila e testa os 5 targets e não publica nada.

---

### Task 1: Podar workflows legados quebrados

Os workflows herdados do upstream referenciam coisas que não existem mais (`codegraph-api`, Docker, FAISS, `libssl-dev`, cargo-fuzz, miri, ambientes `staging`/`production` com URLs de exemplo). Só `ci.yml`, `security.yml`, `semantic-pr.yml` e `release-please.yml` valem a pena.

**Files:**
- Delete: `.github/workflows/{bench,benchmark,ci-enhanced,deploy,deploy-enhanced,docs,nightly,performance-validation,rollback}.yml`
- Modify: `.github/workflows/ci.yml` (remove os passos `libssl-dev`/`pkg-config`, `Run tests` duplicado sem `--all-features`; testes pulam os 5 watchers do WSL só localmente, no CI seguem rodando)
- Create: `scripts/ci/check-workflows.sh`

**Interfaces:**
- Produces: `scripts/ci/check-workflows.sh` (exit 0/1), reutilizado por Tasks 3, 4, 6 como verificação dos YAMLs.

- [ ] **Step 1: Write the failing check.** `scripts/ci/check-workflows.sh` (bash, `set -euo pipefail`): (a) todo `.github/workflows/*.yml` e `action.yml` parseia (`python3 -c 'import yaml…'`, só como validador local); (b) `grep -rn -E 'codegraph-api|libssl-dev|libfaiss|DOCKER_' .github action.yml` não encontra nada; imprime cada violação e sai 1.
- [ ] **Step 2: Run** `bash scripts/ci/check-workflows.sh`. Expected: exit 1, listando `release.yml`, `ci.yml` etc.
- [ ] **Step 3: `git rm`** os 9 workflows listados e editar `ci.yml` como descrito. `release.yml` fica para a Task 3 (ainda reprova o check até lá; marcar o check dele como esperado só na Task 3).
- [ ] **Step 4: Run** o check. Expected: só `release.yml` reprova.
- [ ] **Step 5: Commit** `chore(ci): remove legacy workflows that reference removed crates`.

### Task 2: Metadados de versão e `--version`

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] repository = "https://github.com/isaiaspereira307/codegraph-rust"`), `crates/codegraph-mcp-server/Cargo.toml` (`version.workspace = true`, `repository.workspace = true`, `description`, `readme`)
- Test: `crates/codegraph-mcp-server/tests/version_test.rs`

**Interfaces:**
- Produces: `codegraph --version` imprime `codegraph 1.0.0` (a versão do workspace); Task 3 compara a tag `v1.0.0` com essa saída.

- [ ] **Step 1: Write the failing test** `version_prints_workspace_version`: executa o binário (`env!("CARGO_BIN_EXE_codegraph")`) com `--version` e afirma `stdout.trim() == format!("codegraph {}", env!("CARGO_PKG_VERSION"))` e `env!("CARGO_PKG_VERSION") == "1.0.0"`.
- [ ] **Step 2: Run** `cargo test -p codegraph-mcp-server --test version_test`. Expected: FAIL (`0.1.0`).
- [ ] **Step 3: Alinhar** `version`/`repository` ao workspace; confirmar que o clap do `codegraph` usa `#[command(version)]`.
- [ ] **Step 4: Run** o teste. Expected: PASS.
- [ ] **Step 5: Commit** `chore: align codegraph-mcp-server version and repository with the workspace`.

### Task 3: Workflow de release

**Files:**
- Rewrite: `.github/workflows/release.yml`
- Create: `scripts/release/package.sh`
- Test: `scripts/release/test-package.sh`

**Interfaces:**
- Consumes: `scripts/ci/check-workflows.sh` (Task 1), `codegraph --version` (Task 2).
- Produces: `scripts/release/package.sh <version> <target> <binary-path> <out-dir>` → cria `<out-dir>/codegraph-<version>-<target>.tar.gz|zip` e `<out-dir>/<asset>.sha256`; o release publica também `SHA256SUMS` (concatenação). Tasks 4, 5 e 7 dependem desses nomes.

- [ ] **Step 1: Write the failing test** `scripts/release/test-package.sh`: cria um "binário" falso, roda `package.sh 1.2.3 x86_64-unknown-linux-gnu <fake> <tmp>` e afirma: o tar existe, contém `codegraph-1.2.3-x86_64-unknown-linux-gnu/{codegraph,LICENSE-MIT,LICENSE-APACHE,README.md}`, o `.sha256` confere com `sha256sum -c`; para `x86_64-pc-windows-msvc` gera `.zip` com `codegraph.exe`.
- [ ] **Step 2: Run** `bash scripts/release/test-package.sh`. Expected: FAIL (script ausente).
- [ ] **Step 3: Implement `package.sh`** (bash; `shasum -a 256` com fallback para `sha256sum`; `zip` no Windows via `7z`/`zip` disponível no runner).
- [ ] **Step 4: Run** o teste. Expected: PASS.
- [ ] **Step 5: Reescrever `release.yml`:** gatilhos `push: tags: ['v*']` e `workflow_dispatch` (input `publish: boolean`, padrão `false`, e o dry-run usa a versão do `Cargo.toml`). Job `build` em matriz com runners nativos: `ubuntu-latest`, `ubuntu-24.04-arm`, `macos-15-intel`, `macos-latest`, `windows-latest`; `cargo build --release --locked -p codegraph-mcp-server --bin codegraph --all-features --target <target>`; passos: (i) `codegraph --version` bate com a tag sem o `v`; (ii) smoke test: em diretório temporário, `git init`, um arquivo `.rs`, `codegraph build .` e `codegraph graph-status` saem 0; (iii) no Linux, `ldd` não lista `libssl`; (iv) `package.sh`; (v) `upload-artifact`. Job `publish` (`needs: build`, só em tag ou `publish: true`): baixa artefatos, gera `SHA256SUMS`, notas = seção da versão no `CHANGELOG.md` (fallback: `git log` desde a tag anterior), `gh release create "$TAG" --verify-tag --notes-file … <assets>`. Sem Docker e sem `publish-crates`.
- [ ] **Step 6: Run** `bash scripts/ci/check-workflows.sh`. Expected: exit 0.
- [ ] **Step 7: Commit** `ci: rebuild release workflow for native binaries on five targets`.

### Task 4: Instalador `install.sh`

**Files:**
- Create: `install.sh`
- Test: `scripts/release/test-install.sh`
- Modify: `install-codegraph.sh` (mantém a instalação por fonte; troca a mensagem para citar `install.sh`)

**Interfaces:**
- Consumes: nomes dos assets e `SHA256SUMS` da Task 3.
- Produces: `install.sh` com variáveis `CODEGRAPH_VERSION` (padrão: `latest`), `CODEGRAPH_INSTALL_DIR` (padrão `$HOME/.local/bin`), `CODEGRAPH_RELEASE_BASE_URL` (padrão `https://github.com/isaiaspereira307/codegraph-rust/releases`; aceita `file://` para teste). A Action (Task 7) chama este script.

- [ ] **Step 1: Write the failing tests** em `test-install.sh`, usando um release falso em diretório temporário (gerado com `package.sh`) e `CODEGRAPH_RELEASE_BASE_URL=file://…`: (a) instala em `"$tmp/my bin"` e `"$tmp/my bin/codegraph" --version` roda (binário falso é um script que imprime `codegraph 1.2.3`); (b) `.sha256`/`SHA256SUMS` adulterado → exit 1, destino vazio; (c) `uname -s` forçado para `FreeBSD` (via `CODEGRAPH_TEST_UNAME_S`) → exit 1 e mensagem contém `x86_64-unknown-linux-gnu`.
- [ ] **Step 2: Run** `bash scripts/release/test-install.sh`. Expected: FAIL (script ausente).
- [ ] **Step 3: Implement `install.sh`:** detecta OS/arch → target; resolve `latest` via redirect de `/releases/latest` (sem API autenticada); baixa asset e `SHA256SUMS` com `curl -fsSL` (fallback `wget`), verifica checksum antes de extrair, extrai em diretório temporário e só então `mv` para o destino; avisa se o destino não está no `PATH`. Windows: o script recusa com mensagem apontando o `.zip` do release.
- [ ] **Step 4: Run** o teste. Expected: PASS nos três casos.
- [ ] **Step 5: Commit** `feat: add curl-pipe installer that verifies release checksums`.

### Task 5: Fórmula Homebrew

**Files:**
- Create: `packaging/homebrew/codegraph.rb.tmpl`, `scripts/release/render-formula.sh`
- Test: `scripts/release/test-formula.sh`
- Modify: `.github/workflows/release.yml` (job `homebrew`)

**Interfaces:**
- Consumes: `SHA256SUMS` e nomes de asset (Task 3).
- Produces: `scripts/release/render-formula.sh <version> <SHA256SUMS-path>` → fórmula `codegraph.rb` no stdout, com `url`/`sha256` por plataforma (`on_macos`/`on_linux`, `on_arm`/`on_intel`), `bin.install "codegraph"` e `test do system "#{bin}/codegraph", "--version" end`.

- [ ] **Step 1: Write the failing test:** `test-formula.sh` gera um `SHA256SUMS` falso com 4 assets (sem Windows), renderiza, e afirma: contém as 4 URLs `…/download/v1.2.3/codegraph-1.2.3-<target>.tar.gz` com os hashes certos e `ruby -c` aceita a sintaxe (pula `ruby -c` com aviso se `ruby` ausente).
- [ ] **Step 2: Run.** Expected: FAIL (script ausente).
- [ ] **Step 3: Implement** o template e o renderizador (`sed` sobre placeholders `@VERSION@`, `@SHA_<TARGET>@`; falha com exit 1 se algum hash faltar).
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Job `homebrew`** no `release.yml` (só em tag; `needs: publish`): faz checkout de `isaiaspereira307/homebrew-codegraph` com `secrets.HOMEBREW_TAP_TOKEN`, escreve `Formula/codegraph.rb`, commita e dá push. Se o secret não existe, o job termina com aviso (`if: ${{ secrets.HOMEBREW_TAP_TOKEN != '' }}` via variável de ambiente intermediária) em vez de reprovar o release.
- [ ] **Step 6: Run** `scripts/ci/check-workflows.sh`. Expected: exit 0.
- [ ] **Step 7: Commit** `feat(release): render and publish Homebrew formula`.

### Task 6: Subcomando `codegraph pr-report`

**Files:**
- Create: `crates/codegraph-mcp-server/src/pr_report.rs`
- Modify: `crates/codegraph-mcp-server/src/bin/codegraph.rs` (variante `PrReport` em `Commands`, ao lado de `DetectChanges`, ~linha 170 e 682), `crates/codegraph-mcp-server/src/lib.rs` (`pub mod pr_report;`)
- Test: `crates/codegraph-mcp-server/tests/pr_report_test.rs`, fixture `crates/codegraph-mcp-server/tests/fixtures/detect_changes_sample.json` (saída real de `codegraph detect-changes` num repo fixture, depois de rodar `build`)

**Interfaces:**
- Consumes: resultado JSON de `detect_changes` (campos `risk_score`, `risk_signals`, `review_priorities`, arquivos e funções alterados, `changed_functions`; confirmar nomes exatos na fixture).
- Produces:
  - `pub const REPORT_MARKER: &str = "<!-- codegraph-report -->";`
  - `pub fn render_report(changes: &serde_json::Value, max_bytes: usize) -> String`
  - `pub fn risk_level(score: f64) -> RiskLevel` (`Low < High (≥0.70) < Critical (≥0.85)`)
  - CLI: `codegraph pr-report --base <ref> [--fail-on-risk none|high|critical] [--output <path>] [--project <dir>]`; escreve o markdown em `--output` (ou stdout); exit 0 ok, 3 se o risco atingiu `--fail-on-risk`, 1 se a análise falhou.

- [ ] **Step 1: Write the failing tests:** `risk_level(0.69) == Low`, `(0.70) == High`, `(0.85) == Critical`; `render_report` da fixture começa com `REPORT_MARKER` e contém o score e o primeiro item de `review_priorities`; `render_report` de uma entrada sem arquivos alterados contém "nenhuma mudança analisada" e nenhum nível de risco; entrada gigante (1 MB de funções) devolve ≤ `max_bytes` e contém uma nota "cortado"; teste de CLI (`assert_cmd`/`Command`): repo fixture sem `.code-review-graph/` → `pr-report --base HEAD~1` sai 1 sem escrever arquivo; com grafo e risco acima do limite → sai 3 e ainda escreve o relatório.
- [ ] **Step 2: Run** `cargo test -p codegraph-mcp-server --test pr_report_test`. Expected: FAIL (módulo ausente).
- [ ] **Step 3: Implement** `pr_report.rs` com as assinaturas acima (markdown: cabeçalho com nível de risco, tabela de prioridades de revisão, funções alteradas sem teste, arquivos impactados, `<details>` para o resto); a CLI chama o mesmo caminho do executor que `detect-changes` (não duplica a lógica), com churn ligado como no MCP.
- [ ] **Step 4: Run** os testes. Expected: PASS; depois `cargo clippy -p codegraph-mcp-server --all-targets -- -D warnings` sem avisos novos neste crate.
- [ ] **Step 5: Commit** `feat: add pr-report subcommand with risk gate`.

### Task 7: GitHub Action composta

**Files:**
- Create: `action.yml`, `.github/workflows/pr-review.yml` (dogfood), `scripts/action/upsert-comment.sh`
- Test: `scripts/action/test-upsert-comment.sh`

**Interfaces:**
- Consumes: `install.sh` (Task 4, via `CODEGRAPH_VERSION`), `codegraph build|update|pr-report` (Task 6), `scripts/ci/check-workflows.sh`.
- Produces: Action com inputs `github-token` (obrigatório), `comment` (`true`), `fail-on-risk` (`none`), `version` (`latest`), `details`/`python-version` **não existem**; output `comment-file`. `scripts/action/upsert-comment.sh <comment-file>` usa `gh api` com `GH_TOKEN`, `GITHUB_REPOSITORY`, `PR_NUMBER`.

- [ ] **Step 1: Write the failing test** `test-upsert-comment.sh`: coloca um `gh` falso no `PATH` (grava as chamadas) e afirma: sem comentário existente com o marcador de um autor `Bot` → uma chamada `POST`; com um existente → uma chamada `PATCH` no id dele; comentário com marcador de autor `User` é ignorado (POST); falha do `gh` (403, fork) → imprime `::warning::` e sai 0.
- [ ] **Step 2: Run** `bash scripts/action/test-upsert-comment.sh`. Expected: FAIL (script ausente).
- [ ] **Step 3: Implement** `upsert-comment.sh` portando a lógica de autor/marcador do `action.yml` do CRG (linhas "Upsert sticky PR comment").
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Escrever `action.yml`** (composite): instala o binário por `install.sh` em `$RUNNER_TEMP/codegraph-bin` e o põe no `GITHUB_PATH`; `actions/cache` de `.code-review-graph` com chave `codegraph-schema15-${{ runner.os }}-${{ hashFiles('**/Cargo.lock', '**/package-lock.json', '**/uv.lock', '**/go.sum') }}`; resolve `CODEGRAPH_BASE` (`origin/$BASE_REF` ou `HEAD~1`); `update` se há `graph.db`, com fallback para `build`; `codegraph pr-report --base … --output $RUNNER_TEMP/codegraph-comment.md` (sem `|| true`; falha de análise reprova o job); upsert condicional a `inputs.comment == 'true'` e evento `pull_request`; gate de risco repassa `--fail-on-risk` (o exit 3 reprova o job depois de comentar). Mensagens de erro dizem "este PR NÃO foi analisado" no caso de exit 1.
- [ ] **Step 6: `pr-review.yml`:** roda `uses: ./` em `pull_request`, `permissions: contents: read, pull-requests: write`, com `fetch-depth: 0`; falha só depois do release existir (a Action depende de um release publicado, então o workflow nasce com `if: ${{ vars.CODEGRAPH_DOGFOOD == 'true' }}`).
- [ ] **Step 7: Run** `scripts/ci/check-workflows.sh`. Expected: exit 0.
- [ ] **Step 8: Commit** `feat: add composite GitHub Action for risk-scored PR comments`.

### Task 8: Documentação, fechamento e primeiro release

**Files:**
- Modify: `README.md` (seção Install: `curl`, Homebrew, binários, Action, fonte), `docs/INSTALLATION_GUIDE.md` e `docs/USAGE_GUIDE.md` (reescritos curtos, sem agentic/embeddings, ou removidos se o README cobre), `docs/README.md` (tirar a seção "Legacy" correspondente), `CLAUDE.md` (lista "Stale references"), `CHANGELOG.md`, `PLANO.md` (Próximos passos item 3 → ✅; Fase 5 "Distribuição"), `CONTRIBUTING.md` (como cortar um release)
- Create: `docs/GITHUB_ACTION.md`

- [ ] **Step 1:** Reescrever as docs listadas; cada comando citado precisa existir (`codegraph --help`, `bash install.sh` com release falso).
- [ ] **Step 2: Verificação final local:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` (reportar os avisos antigos de `codegraph-core` sem mascarar), `cargo test -p codegraph-mcp-server --test version_test --test pr_report_test`, e os `scripts/**/test-*.sh` e `scripts/ci/check-workflows.sh`. Expected: tudo verde.
- [ ] **Step 3: Commit** `docs: document install options and the GitHub Action; close distribution`.
- [ ] **Step 4: Pedir confirmação ao usuário** para as ações externas: push na `main`, dry-run do release (`workflow_dispatch`, `publish=false`), criar `isaiaspereira307/homebrew-codegraph` + secret `HOMEBREW_TAP_TOKEN`, e a tag `v1.0.0`. Executar só o que for aprovado; depois do dry-run verde, conferir que os 5 assets e `SHA256SUMS` existem.
- [ ] **Step 5:** Rodar `cargo clean` (memória do projeto: `target/` chegou a 39 GB).
