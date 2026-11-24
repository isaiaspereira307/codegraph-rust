# Cache do AnalysisGraph e do churn — design curto

## Intenção

Ferramentas de análise e de review ficam mais rápidas em chamadas repetidas, sem mudar nenhum
resultado. Hoje cada chamada de análise recarrega o `AnalysisGraph` do SQLite dentro do
`with_conn` de conexão única (serializando as outras), e cada review roda `git log` de novo
(até 5 s) mesmo com o HEAD igual.

## Metas

- `AnalysisGraph` em cache, chaveado pelo marcador de mudança do banco (`PRAGMA data_version` +
  `total_changes()`), como o `Snapshot` já faz em `GraphQuery`.
- Churn em cache por (raiz, HEAD), no máximo 32 entradas, como o CRG; timeout do `git log`
  lembrado de forma persistente (o processo não paga os 5 s de novo).
- Chaves de caminho do churn cobrem a raiz canônica (raiz com symlink deixa de zerar o churn).

## Não-metas / anti-metas

- Nenhum score, ranking ou resultado de ferramenta muda. Mudanças de score (fallback de
  TESTED_BY por nome solto, isenção de migrations, fallback `0.05·flow_count`) ficam em plano
  próprio, com medição de paridade contra o CRG antes e depois.
- Sem invalidação por tempo; sem cache entre processos.

## Abordagem (escolhida pelo usuário)

Só desempenho agora, scores depois. Alternativa descartada: tudo num plano só, o que misturaria
"mais rápido" com "score diferente" e impediria atribuir regressões.

## Decisões minhas

- O cache do `AnalysisGraph` mora em `GraphQuery` (clones o compartilham), ao lado do cache do
  `Snapshot`; o carregamento do pós-processo (`analysis/mod.rs`) continua sem cache.
- Só o **timeout** do `git log` é persistente. Repositório sem commits ou `git` ausente falha
  rápido e não é lembrado, para um repositório que ganha o primeiro commit voltar a funcionar sem
  reiniciar o servidor. `clear_churn_cache()` zera tudo (testes, processos longos).
- O HEAD vem de `git rev-parse HEAD` sob o mesmo orçamento de 5 s.
