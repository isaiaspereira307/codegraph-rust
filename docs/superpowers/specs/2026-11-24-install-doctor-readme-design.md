# Instalação, `uninstall`, `doctor` e README — design

## Intenção

Outra pessoa precisa conseguir instalar o `codegraph` num projeto, saber se está funcionando e
desfazer tudo, sem ler o código. Hoje só existe `codegraph init` (hooks por projeto para
Claude Code e Codex, bloco de instruções, índice). Faltam desfazer, diagnóstico, as skills de
revisão instaladas e um README que descreva o produto atual (SQLite, sem LLM, sem embeddings).

## Metas

- `init` continua sendo a entrada de instalação, sempre por projeto.
- `uninstall` remove só o que o `init` criou.
- `doctor` diz o que está quebrado e como consertar, com código de saída útil em CI.
- README curto e verdadeiro; tudo que descreve SurrealDB, embeddings, LLM e ferramentas
  agênticas sai.

## Não-metas

- Outras plataformas além de Claude Code e Codex (decisão do usuário).
- Instalação global ou em `~/.claude` / `~/.codex` (regra do repo: sem hooks no nível de usuário).
- Reescrever `docs/` inteiro; só o que o README referencia e links quebrados.

## Anti-metas

- Apagar ou reescrever configuração do usuário que o `init` não criou.
- `doctor` que modifica algo, ou que depende de rede.
- README que prometa ferramentas ou plataformas que não existem.

## Abordagens consideradas

A. Por projeto, em cima do `init` — **escolhida pelo usuário**. B. `install` global como o do CRG:
descartada, mexe em settings globais (onde o CRG v9 já brigou com o Rust) e exige backup/undo
mais cuidadoso. C. Só `doctor`: descartada, não entrega "instalável por outra pessoa".

## Decisões do usuário

- Só Claude Code e Codex.
- MCP opcional e documentado: o `init` **não** registra o servidor MCP por padrão.

## Design

### `init` (mudanças)

1. Skills de revisão (`rules-for-claude-code/skills/{review-changes,review-pr,review-delta}`)
   embutidas no binário (`include_str!`) e copiadas para `<projeto>/.claude/skills/<nome>/SKILL.md`
   quando a seleção inclui Claude. Nunca sobrescreve um diretório de skill que o `init` não criou
   (marcador `.codegraph-managed` dentro do diretório). Codex não ganha skills (minha decisão:
   ele continua só com hooks + bloco de instruções em `AGENTS.md`, como hoje).
2. Flag opcional `--mcp`: grava a entrada `codegraph` em `.mcp.json` (Claude, chave `mcpServers`,
   comando `codegraph start stdio`) e, para o Codex, em `.codex/config.toml`
   (`[mcp_servers.codegraph]`). Mescla sem tocar em outras entradas; recusa symlink e JSON/TOML
   inválido como o código atual de hooks. Sem a flag nada de MCP é escrito.
3. Tudo continua validado antes de qualquer escrita (padrão atual de `prepare`).

### Decisões minhas (resolvem as dúvidas do revisor; corrija se discordar)

- **Sem manifesto de instalação.** `uninstall` só remove o que é reconhecível como do CodeGraph:
  bloco marcado `<!-- codegraph:begin/end -->`, entradas de hook cujo comando é o `HOOK_COMMAND`,
  skills com `.codegraph-managed`, entrada `codegraph` em `.mcp.json`/`.codex/config.toml`, bloco
  marcado do hook de git. Funciona também em projetos inicializados por versões antigas. Um
  arquivo só é apagado se, depois da remoção, ficar vazio (`{}` ou sem texto); se o usuário
  guardou outro conteúdo nele, fica. Não distinguimos "criado pelo init" de "já existia".
- **Escopo de `--hooks` no `uninstall`** (padrão `both`): vale para hooks, bloco de instruções,
  entrada MCP e skills do harness escolhido (skills só existem para Claude). O hook de git não
  depende do harness: é removido sempre, e só o bloco marcado.
- **`init --mcp`** age sobre os harnesses escolhidos por `--hooks` (ou pelo prompt). Com
  `--hooks none`, `--mcp` é erro pedindo `--hooks claude|codex|both`.
- **Hook de git**: o `init` não o instala (continua opt-in via `codegraph hooks git install`);
  `doctor` só informa; `uninstall` remove o bloco marcado se existir.
- **`doctor`, CRG**: versão via `code-review-graph --version` (ausente do PATH = `ok`, nada a
  checar); os hooks globais são detectados lendo `~/.claude/settings.json` por comandos que
  contêm `code-review-graph`; o limiar 2.4.0 vem de `PLANO.md` (2.3.7 grava schema 9, 2.4.0
  grava 15). "Hook presente" = comando igual ao `HOOK_COMMAND` e `codegraph` resolvível no PATH.
- **`--keep-index` removido**: o índice é mantido por padrão; só `--purge-index` o apaga.

### `codegraph uninstall [path] [--hooks claude|codex|both] [--purge-index] [--dry-run]`

Remove, por projeto: entradas de hook do CodeGraph em `.claude/settings.json` e `.codex/hooks.json`
(preservando o resto), blocos `<!-- codegraph:begin/end -->` de `CLAUDE.md`/`AGENTS.md` (apaga o
arquivo só se ficar vazio e foi criado pelo `init`), skills com marcador, entrada `codegraph` do
`.mcp.json` e `.codex/config.toml`, o bloco do hook de git (`codegraph hooks git uninstall`).
Apaga `.code-review-graph/` só com `--purge-index` (o `graph.db` pode ser do CRG; padrão é manter).
`--dry-run` lista o que faria. Idempotente.

### `codegraph doctor [path] [--json]`

Somente leitura, sem rede. Cada verificação devolve `ok | warn | fail` e, se não for `ok`, a
correção em uma linha. Verificações:

1. Binário: caminho de `codegraph` no PATH, versão, e se difere do executável em uso.
2. CRG instalado no PATH: versão; `warn` se for < 2.4.0 (grava schema 9) e os hooks globais dele
   (`~/.claude/settings.json`) rodam neste projeto sem `.code-review-graph/crg-hooks-off`.
3. `graph.db`: existe, abre, `schema_version` = 15, `metadata.build_state`, contagens de nós/
   arestas, idade do `last_updated`, sufixo `@L` residual em nós `File` (aviso de índice antigo,
   manda rodar `codegraph build`).
4. Hooks do projeto (Claude/Codex): presentes e apontando para um `codegraph` existente.
5. Hook de git pós-commit: instalado/ausente (informativo).
6. Skills instaladas: presentes com marcador.
7. MCP: entradas em `.mcp.json`/`.codex/config.toml` (informativo) e `codegraph` resolvível pelo
   comando registrado.
8. Lock: `write.lock` presa por processo vivo (informativo).

Saída: tabela legível; `--json` para máquinas. Código de saída 0 sem `fail`, 1 com `fail`.

### README

Reescrito do zero (~250 linhas): o que é, instalação (`cargo install`, release), início rápido
(`codegraph init`, `build`, `tool run`), lista de ferramentas (gerada de `codegraph tool list`),
MCP opcional (`init --mcp` e configuração manual para Claude e Codex), `doctor`/`uninstall`,
convivência com o CRG, linguagens suportadas (remete a `docs/SUPPORTED_LANGUAGES.md`), tiers,
licença. Os `install-codegraph*.sh` e `docs/README.md` ganham o aviso de legado ou são corrigidos
quando contradizem o README. `CLAUDE.md` perde as linhas de "stale references" que ficarem falsas.

## Fica para o implementador

Formato exato do JSON de `doctor --json`; como detectar "hook do CodeGraph" em `settings.json`
(reaproveitar `merge_settings`/`HOOK_COMMAND`); texto das mensagens; se o `.codex/config.toml`
de projeto é lido pelo Codex (confirmar na documentação do Codex; se não for, o `--mcp` do Codex
só imprime o trecho para colar em `~/.codex/config.toml`).

## Testes esperados

Testes de integração em `crates/codegraph-mcp-server/tests/` com projeto temporário: `init` →
`uninstall` devolve a árvore ao estado inicial (incluindo arquivos pré-existentes do usuário
preservados); `init --mcp` mescla sem apagar outras entradas; `doctor` em projeto limpo, com
`graph.db` v9, com `@L` residual e sem hooks.
