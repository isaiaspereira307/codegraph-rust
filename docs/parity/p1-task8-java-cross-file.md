# P1 Task 8 — Java entre arquivos, anotações e emissores de framework

Port dos resolvers de receptor tipado do CRG (`_java_same_package_file`,
`_resolve_typed_method_target`) e dos emissores que o harness confirma como arestas reais:
`@Value`, `@KafkaListener`, Temporal, `@EventListener`, `publishEvent` e rotas funcionais
WebFlux. Medido em 2026-11-25 com CRG `bbdda4b`, binário debug, tier `fast` padrão.
Limites congelados em [`p1-baseline.md`](p1-baseline.md).

## Vermelho e decisões de escopo

O probe `scripts/parity/probes/java-cross-file/` foi medido antes do port com o binário de
`a137eca` (`/tmp/opencode/p1-t8-red.md`) e o probe de framework em
`/tmp/opencode/p1-t8-framework-run.md`. O plano pedia INHERITS e INJECTS no probe, mas nenhum
dos dois existia no corpus; adicionei `Payment`/`StripePayment`/`Billing` para que as duas arestas
passassem a existir no CRG antes de serem comparadas — não são emissores novos.

Achados do harness que **não** viraram emissor:

- `@Value` em componente de `record` não emite `DEPENDS_ON_CONFIG` no CRG: o emissor percorre
  `field_declaration`, não record components. O probe usa um campo normal.
- `REFERENCES` permanece desligado no `fast` também para Java.
- Resolução de tipo não usa `IMPORTS_FROM` como prova de binding: `import static`, wildcard e
  JDK continuam como texto solto.

## Resolução entre arquivos

- `receiver_type` e `receiver_resolution` agora sobrevivem ao parser; um receptor com tipo
  explícito (`Cart cart`, `com.x.Cart cart`) usa `java_call_target` antes do `exact` textual.
- `java_type_file` replicated the CRG rule for FQN, simple imported type and same-package type.
  `receiver_form=expression` não é atribuído ao arquivo local: o alvo fica solto, como no CRG.
- `new Event()` emite `Calls` marcado `java_construction`; `java_construction_target` liga ao nó
  da classe em `src/main/java`, `src/test/java` ou mesmo pacote.
- `resolve_spring_events` reconstrói as arestas `PUBLISHES`/`HANDLES` do publisher ao listener
  com `resolution=spring_application_event`, `confidence=0.95` e `confidence_tier=INFERRED`.
  A reconciliação recalcula essas arestas a cada snapshot, então um listener apagado não deixa
  `CALLS` órfão.
- `java_call_target` ignora arestas já marcadas `spring_resolved`; isto evita ambiguidade quando
  o evento e o método publicado têm o mesmo nome simples.

## Emissores de framework confirmados pelo harness

| emissor | aresta / nó | evidência |
|---|---|---|
| `@Value("${...}")` | `DEPENDS_ON_CONFIG -> config:<key>` | probe de framework |
| `@KafkaListener(topics=...)` | `CONSUMES -> kafka:<topic>` | CRG `bbdda4b`, já presente no `java-cross-file` |
| `@KafkaListener` + `kafkaTemplate.send` | `PRODUCES -> kafka:<topic>` | probe de framework |
| `@Scheduled(cron=...)` | `TRIGGERS -> cron:<expr>` | probe de framework |
| `@TemporalWorkflow` / `@WorkflowMethod` | `TEMPORAL_STUB` | probe de framework |
| `@EventListener` | nó virtual `Event` + `HANDLES` | probe de framework |
| `publishEvent(new Event())` | `PUBLISHES` + `CALLS` ao construtor e ao listener | probe de framework |
| `route().GET(path, handler::method)` | nó `Endpoint` + `HANDLES` do handler real | probe de framework |

`is_framework_edge` mantém as arestas acima quando o alvo é textual (por exemplo
`kafka:probe`), preservando o comportamento do CRG. Os metadados de linha do `PUBLISHES`
são usados para manter o `line agreement` do `CALLS` inferido em 1.000.

## Medição verde

Comandos (o probe de framework vive em `scripts/parity/probes/java-framework/`):

```bash
cargo build -q -p codegraph-mcp-server --bin codegraph
python3 scripts/parity/parity.py run scripts/parity/probes/java-cross-file \
  --work /tmp/opencode/p1-t8-cross3 \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t8-cross3.json --md /tmp/opencode/p1-t8-cross3.md
python3 scripts/parity/parity.py run scripts/parity/probes/java-framework \
  --work /tmp/opencode/p1-t8-fw6 \
  --crg-repo /tmp/opencode/parity-p1-t2/work/crg-repo \
  --codegraph target/debug/codegraph \
  --json /tmp/opencode/p1-t8-fw6.json --md /tmp/opencode/p1-t8-fw6.md
```

### `java-cross-file`

| métrica | valor |
|---|---|
| nós | 30 / 30 — Jaccard 1.000 |
| CALLS | 8 / 8 — 1.000 (resolvido 1.000, linha 1.000) |
| CONSUMES | 1 / 1 — 1.000 |
| CONTAINS | 20 / 20 — 1.000 |
| IMPORTS_FROM | 7 / 7 — 1.000 |
| INHERITS | 1 / 1 — 1.000 |
| INJECTS | 1 / 1 — 1.000 |
| flows | 5 / 5 — entry 1.000, membros 1.000 |
| comunidades | 6 / 6 — ARI 1.000 |

### `java-framework`

| métrica | valor |
|---|---|
| nós | 31 / 31 — Jaccard 1.000 |
| CALLS | 7 / 7 — 1.000 (linha 1.000) |
| CONTAINS | 18 / 18 — 1.000 |
| DEPENDS_ON_CONFIG | 2 / 2 — 1.000 |
| HANDLES | 3 / 3 — 1.000 |
| IMPORTS_FROM | 9 / 9 — 1.000 |
| PUBLISHES | 1 / 1 — 1.000 |
| TEMPORAL_STUB | 1 / 1 — 1.000 |
| TRIGGERS | 1 / 1 — 1.000 |
| flows | 2 / 2 — entry 1.000, membros 1.000 |
| comunidades | 6 / 6 — ARI 1.000 |

O corpus Java completo (`python3 scripts/parity/parity.py corpus`) também fica em 1.000 em
nós, CALLS, IMPORTS_FROM, INHERITS, INJECTS, flows e ARI. Os dois probes entraram em
`p1_snapshot.sh` e `check_p1.py`: o snapshot completo `/tmp/opencode/parity-p1-t8` passou com
**111 pisos e nenhuma métrica abaixo do congelado**.

## Validação

- `cargo check --workspace` limpo.
- `cargo test -q -p codegraph-parser --lib`: 117 testes, 0 falhas.
- `cargo test -q -p codegraph-mcp --lib`: 105 testes, 0 falhas.
- `cargo test -q -p codegraph-mcp --test reconciliation_test`: testes novos de eventos Spring e
  rota WebFlux verdes; a suíte completa ainda carrega a falha pré-existente documentada em
  `p1-task5-rust-resolution.md`.
- `python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1-t8`: 111 pisos OK.
