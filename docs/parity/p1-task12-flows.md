# P1 Task 12 — flows com ID estável e escrita incremental

`store_flows` deixou de apagar e reinserir `flows`/`flow_memberships` a cada `finalize`.
`reconcile_flows` (`crates/codegraph-graph/src/analysis/flows/mod.rs`) compara os flows traçados
com as linhas gravadas e escreve só a diferença. O traço e a criticalidade continuam completos
(ruling R15); nomes, criticalidades e `path_json` são os mesmos bytes de antes.

## Regra de identidade (ruling R14)

- Chave do flow: `nodes.qualified_name` do `entry_point_id` (rowids de nó podem mudar num rebuild,
  o nome qualificado não). `flows.id` continua `AUTOINCREMENT`; não há hash para inteiro.
- Chave já gravada e traçada de novo: mantém o id. A linha (com `updated_at = datetime('now')`) e
  as memberships só são reescritas se `name`, `entry_point_id`, `depth`, `node_count`,
  `file_count`, `criticality` ou `path_json` mudaram; senão nada é tocado.
- Chave gravada que não foi traçada: o flow e as memberships são apagados.
- Chave nova: `INSERT`, na ordem de `flows` (criticalidade decrescente). Num banco vazio os ids
  saem iguais aos da versão antiga (1..n por criticalidade), como no CRG.
- Duplicatas: entre linhas gravadas com a mesma chave (ou cujo nó de entrada sumiu) fica a de menor
  id e as outras são apagadas; um flow traçado cuja chave já foi usada por um anterior em `flows`
  é inserido como novo. Como `qualified_name` é `UNIQUE` e há um flow por entry point, o segundo
  caso não ocorre hoje; a regra só garante determinismo sem perder flows.
- `DELETE FROM metadata WHERE key = 'csharp_flows_dirty'` segue igual. `store_flows` ainda devolve
  o número de flows; `PostprocessStats.flow_writes` traz inserted/updated/deleted/unchanged e o log
  `analysis stored` ganhou `flows_store_s` e esses contadores.

Um rebuild após mudança de `GRAPH_SHAPE_VERSION` também mantém os ids enquanto o nome qualificado
do entry point for o mesmo; só reescreve as linhas cujo conteúdo mudou.

## Medição neste repositório

Binário debug (`cargo build -p codegraph-mcp-server --bin codegraph`), `RUST_LOG=info codegraph
build .`, 3 repetições cada, mediana. "Antes" é o HEAD `4ac9a96` só com o cronômetro
`flows_store_s` acrescentado.

| | `flows_s` (traço) | `flows_store_s` (escrita) | ids iguais entre builds |
|---|---|---|---|
| antes | 0,846 (0,723 / 0,846 / 0,933) | 0,0131 (0,0114 / 0,0131 / 0,0269) | 0 de 355 |
| depois | 0,650 (0,580 / 0,650 / 0,856) | 0,0037 (0,0036 / 0,0037 / 0,0074) | 356 de 356 |

`flows_s` mede só o traço, que não mudou: a diferença é ruído. A escrita caiu ~3,5x porque nos
builds sem mudança de fonte nada é escrito (`unchanged=356`). O primeiro build "depois" pegou a
fonte já alterada por esta task: `inserted=1 updated=1 unchanged=354`.

## Regressão P1

`bash scripts/parity/p1_snapshot.sh` + `python3 scripts/parity/check_p1.py`:
`P1 baseline OK: 151 floors, no metric dropped` (entry points, membros e ARI inclusos).
