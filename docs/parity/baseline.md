# Parity baseline

> Os limites congelados do P1 estão em [`p1-baseline.md`](p1-baseline.md), com detector
> `scripts/parity/check_p1.py`. Este arquivo é o log histórico das fases 2–4.

Linha de base depois do formato de grafo CRG (Fase 2, plano 1: commits d5f4e09..HEAD).
Jaccard por linguagem; "antes" e o baseline do commit 55495fc (mantido abaixo em "Antes").

| linguagem | nos | CALLS | IMPORTS_FROM | CONTAINS | TESTED_BY |
|---|---|---|---|---|---|
| python | 0.512 -> 1.000 | 0.121 -> 0.545 | 0.000 -> 1.000 | 0.000 -> 1.000 | 0.000 -> 0.500 |
| typescript | 0.200 -> 0.438 | 0.000 -> 0.133 | 0.000 -> 1.000 | 0.000 -> 0.308 | 0.000 -> 0.000 |
| go | 0.304 -> 1.000 | 0.162 -> 0.129 | 0.000 -> 1.000 | 0.000 -> 0.932 | n/a |
| rust | 0.383 -> 0.737 | 0.167 -> 0.500 | 0.000 -> 0.500 | 0.000 -> 0.793 | 0.000 -> 0.000 |
| java | 0.169 -> 0.947 | 0.000 -> 0.350 | 0.000 -> 1.000 | 0.000 -> 1.000 | n/a |

Nenhuma linguagem piorou em nos. Unica queda: CALLS de go (0.162 -> 0.129), porque o
resolvedor agora so liga nomes soltos no mesmo arquivo ou via arquivo importado (como o CRG)
e o extrator Go grava textos de chamada/closures como alvo (plano de Go).

Plano TypeScript (`docs/superpowers/plans/2026-11-20-fase2-typescript.md`): typescript passou a
1.000 em nos, CALLS, CONTAINS, IMPORTS_FROM, INHERITS e TESTED_BY (de 0.438 / 0.133 / 0.308 /
1.000 / 0 / 0); REFERENCES continua 0 (fora do escopo). As outras linguagens nao mudaram.
Plano Go (`docs/superpowers/plans/2026-11-20-fase2-go.md`): go passou a 1.000 em nos (sem
divergencia de kind), CALLS e CONTAINS (de 0.129 / 0.932); IMPORTS_FROM segue 1.000. As outras
linguagens nao mudaram. O corpus rust e nao deterministico (divergentes 6/9/12 e CONTAINS
0.576/0.677 entre execucoes iguais: qual dos nos `S` duplicados, struct ou `impl`, ganha o
sufixo `@L` varia); ja acontecia antes deste plano.
Plano Java (`docs/superpowers/plans/2026-11-20-fase2-java.md`): java passou a 1.000 em nos,
CALLS, INHERITS e INJECTS (de 0.947 / 0.350 / 0 / 0); CONTAINS e IMPORTS_FROM seguem 1.000. As
outras linguagens nao mudaram.
Plano Rust (`docs/superpowers/plans/2026-11-20-fase2-rust.md`): rust passou a 1.000 em nos,
CALLS, CONTAINS, IMPLEMENTS, IMPORTS_FROM e TESTED_BY (de 0.737 / 0.500 / 0.677 / 0 / 0.500 /
0); sem nos `impl`, o corpus rust ficou deterministico (duas execucoes iguais). As outras
linguagens nao mudaram.

Plano Python (`docs/superpowers/plans/2026-11-20-fase2-python.md`): python passou a 1.000 em
nos (sem divergentes), CONTAINS, IMPORTS_FROM, INHERITS e TESTED_BY (de 1.000 / 1.000 / 1.000 /
0 / 0.500), e CALLS de 0.545 para 0.923 (resolved 0.833); a unica chamada que falta e
`process_request` -> `AuthService.authenticate`, receptor tipado por anotacao (fora do escopo).
REFERENCES segue 0 no tier padrao (`fast` descarta `References`); com `--index-tier full` e
`CODEGRAPH_ANALYZERS=0` fica 1.000 no corpus e na sonda. Sonda `scripts/parity/probes/python`:
nos, CALLS, IMPORTS_FROM, INHERITS e TESTED_BY 1.000; CONTAINS 0.958 (metodo de classe
aninhada: o CRG grava a origem como texto `F::Inner`, sem no). As outras linguagens nao
mudaram (`is_test` em todo no de arquivo de teste nao gerou divergencia).

Sonda `scripts/parity/probes/python-imports` (imports de pacote/namespace/modulo solto, folha
aninhada ambigua): nos 1.000 (26/26), CONTAINS e IMPORTS_FROM 1.000; CALLS 0.684 (resolvido
0.786) e REFERENCES 0 no tier padrao (caminho pontuado `q.lib.f()` e `mod.Thing().go()` ainda
nao portados).

Fase 4 (`docs/superpowers/plans/2026-11-23-fase4-analise-grafo.md`, 2026-11-24): o harness
compara flows e comunidades. O CRG roda com `--extra communities` (igraph, detector `leiden`);
sem igraph ele cai no agrupamento por diretorio (`file`) e o ARI nao diz nada (na primeira
rodada, sem o extra: ARI 0.101 / 0.340 / 0.206 / 0.445 / 0.304).

| linguagem | CALLS | entry points (CRG/Rust, Jaccard) | membros (media/min) | max \|Δ crit\| | comunidades (CRG/Rust) | ARI |
|---|---|---|---|---|---|---|
| python | 0.923 | 3/4, 0.750 | 0.833 / 0.500 | 0.020 | 8/5 | 0.403 |
| typescript | 1.000 | 2/2, 1.000 | 1.000 / 1.000 | 0.000 | 3/3 | 1.000 |
| go | 1.000 | 13/13, 1.000 | 1.000 / 1.000 | 0.000 | 10/10 | 1.000 |
| rust | 1.000 | 5/5, 1.000 | 1.000 / 1.000 | 0.000 | 7/7 | 1.000 |
| java | 1.000 | 6/6, 1.000 | 1.000 / 1.000 | 0.000 | 7/7 | 1.000 |
| sonda python | 1.000 | 8/8, 1.000 | 1.000 / 1.000 | 0.000 | 8/9 | 1.000 |
| sonda python-imports | 0.684 | 2/2, 1.000 | 0.757 / 0.714 | 0.120 | 4/6 | 0.821 |

Criterio (entry points e membros >= 0.95 onde CALLS e 1.00) atendido em typescript, go, rust,
java e na sonda python. Onde CALLS diverge, flows e comunidades divergem junto: no corpus python
falta `process_request` -> `AuthService.authenticate` (receptor tipado por anotacao), entao
`authenticate` vira entry point so no Rust; na sonda python-imports faltam as chamadas por
caminho pontuado. ARI so registrado (Decisao 3); onde CALLS e igual, a particao Leiden coincide
com a do igraph nesses corpora pequenos. As contagens de comunidades sao linhas de
`communities`; o ARI usa so os nos com `community_id` nos dois lados.

Nao relacionado a Fase 4: o Rust grava nos `Directory` (linguagem vazia, aparecem como
`unknown` abaixo) para a raiz e os subdiretorios, com CONTAINS raiz -> subdiretorio; o CRG nao.
Por isso CONTAINS da sonda python caiu de 0.958 para 0.902 e o da sonda python-imports de 1.000
para 0.909 (o parser nao mudou na Fase 4).

"Depois" abaixo e o relatorio regerado na Fase 4 (Task 13).

# Depois

CRG commit `bbdda4b`; corpus: fixtures from `tests/fixtures`, one repository per language.

### Language corpus: python

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| python | 31 | 31 | 31 | 1.000 | 0 |
| unknown | 0 | 1 | 0 | 0.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 25 | 25 | 24 | 0.923 | 0.833 | 1.000 |
| CONTAINS | 26 | 26 | 26 | 1.000 | 1.000 | 1.000 |
| IMPORTS_FROM | 5 | 5 | 5 | 1.000 | 1.000 | 1.000 |
| INHERITS | 1 | 1 | 1 | 1.000 | 1.000 | 1.000 |
| REFERENCES | 5 | 0 | 0 | 0.000 | 0.000 | - |
| TESTED_BY | 6 | 6 | 6 | 1.000 | 1.000 | 1.000 |

#### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| python | 3 | 4 | 3 | 0.750 | 0.833 / 0.500 | 0.007 / 0.020 |

#### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 8 | 5 | leiden | leiden | 16 | 0.403 |

##### unknown: nodes only in Rust

- `<work>/python/rust`

##### CALLS: edges only in CRG

- `sample_python.py::process_request` -> `sample_python.py::AuthService.authenticate`

##### CALLS: edges only in Rust

- `sample_python.py::process_request` -> `authenticate`

##### REFERENCES: edges only in CRG

- `sample_callback_refs.py::register_keyword_callback` -> `sample_callback_refs.py::keyword_callback`
- `sample_callback_refs.py::trigger_executor` -> `sample_callback_refs.py::executor_callback`
- `sample_callback_refs.py::trigger_filter` -> `sample_callback_refs.py::filter_callback`
- `sample_callback_refs.py::trigger_map` -> `sample_callback_refs.py::map_callback`
- `sample_python.py` -> `sample_python.py::BaseService`

##### python: flow entry points only in Rust

- `sample_python.py::AuthService.authenticate`

### Language corpus: typescript

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| typescript | 16 | 16 | 16 | 1.000 | 0 |
| unknown | 0 | 1 | 0 | 0.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 11 | 11 | 11 | 1.000 | 1.000 | 1.000 |
| CONTAINS | 13 | 13 | 13 | 1.000 | 1.000 | 1.000 |
| IMPORTS_FROM | 4 | 4 | 4 | 1.000 | 1.000 | 1.000 |
| INHERITS | 1 | 1 | 1 | 1.000 | 1.000 | 1.000 |
| REFERENCES | 7 | 0 | 0 | 0.000 | 0.000 | - |
| TESTED_BY | 1 | 1 | 1 | 1.000 | 1.000 | 1.000 |

#### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| typescript | 2 | 2 | 2 | 1.000 | 1.000 / 1.000 | 0.000 / 0.000 |

#### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 3 | 3 | leiden | leiden | 11 | 1.000 |

##### unknown: nodes only in Rust

- `<work>/typescript/rust`

##### REFERENCES: edges only in CRG

- `sample_typescript.ts::UserRepository` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserRepository.findById` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserRepository.save` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserService.createUser` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserService.getUser` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::handleGetUser` -> `Request`
- `sample_typescript.ts::handleGetUser` -> `Response`

### Language corpus: go

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| go | 45 | 45 | 45 | 1.000 | 0 |
| unknown | 0 | 1 | 0 | 0.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 34 | 34 | 34 | 1.000 | 1.000 | 1.000 |
| CONTAINS | 43 | 43 | 43 | 1.000 | 1.000 | 1.000 |
| IMPORTS_FROM | 2 | 2 | 2 | 1.000 | 1.000 | 1.000 |

#### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| go | 13 | 13 | 13 | 1.000 | 1.000 / 1.000 | 0.000 / 0.000 |

#### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 10 | 10 | leiden | leiden | 37 | 1.000 |

##### unknown: nodes only in Rust

- `<work>/go/rust`

### Language corpus: rust

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| rust | 30 | 30 | 30 | 1.000 | 0 |
| unknown | 0 | 1 | 0 | 0.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 20 | 20 | 20 | 1.000 | 1.000 | 1.000 |
| CONTAINS | 27 | 27 | 27 | 1.000 | 1.000 | 1.000 |
| IMPLEMENTS | 1 | 1 | 1 | 1.000 | 1.000 | 1.000 |
| IMPORTS_FROM | 1 | 1 | 1 | 1.000 | 1.000 | 1.000 |
| TESTED_BY | 6 | 6 | 6 | 1.000 | 1.000 | 1.000 |

#### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| rust | 5 | 5 | 5 | 1.000 | 1.000 / 1.000 | 0.000 / 0.000 |

#### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 7 | 7 | leiden | leiden | 23 | 1.000 |

##### unknown: nodes only in Rust

- `<work>/rust/rust`

### Language corpus: java

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| java | 36 | 36 | 36 | 1.000 | 0 |
| unknown | 0 | 1 | 0 | 0.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 15 | 15 | 15 | 1.000 | 1.000 | 1.000 |
| CONTAINS | 34 | 34 | 34 | 1.000 | 1.000 | 1.000 |
| IMPORTS_FROM | 11 | 11 | 11 | 1.000 | 1.000 | 1.000 |
| INHERITS | 3 | 3 | 3 | 1.000 | 1.000 | 1.000 |
| INJECTS | 4 | 4 | 4 | 1.000 | 1.000 | 1.000 |

#### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| java | 6 | 6 | 6 | 1.000 | 1.000 / 1.000 | 0.000 / 0.000 |

#### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 7 | 7 | leiden | leiden | 34 | 1.000 |

##### unknown: nodes only in Rust

- `<work>/java/rust`

# Antes

CRG commit `bbdda4b`; corpus: fixtures from `tests/fixtures`, one repository per language.

### Language corpus: python

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| python | 31 | 31 | 21 | 0.512 | 6 |

### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 25 | 12 | 4 | 0.121 | 0.286 |
| CONTAINS | 26 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 5 | 5 | 0 | 0.000 | 0.000 |
| INHERITS | 1 | 0 | 0 | 0.000 | 1.000 |
| REFERENCES | 5 | 0 | 0 | 0.000 | 0.000 |
| TESTED_BY | 6 | 0 | 0 | 0.000 | 0.000 |

##### python: nodes only in CRG

- `caller_example.py`
- `multi_call_example.py`
- `sample_callback_refs.py`
- `sample_python.py`
- `sample_python.py::AuthService.__init__`
- `sample_python.py::AuthService._validate_token`
- `sample_python.py::AuthService.authenticate`
- `sample_python.py::BaseService.__init__`
- `sample_python.py::BaseService.start`
- `test_sample.py`

##### python: nodes only in Rust

- `caller_example.py::sample_python`
- `sample_callback_refs.py::concurrent.futures`
- `sample_python.py::__init__`
- `sample_python.py::__init__@L20`
- `sample_python.py::_validate_token`
- `sample_python.py::authenticate`
- `sample_python.py::os`
- `sample_python.py::pathlib`
- `sample_python.py::start`
- `test_sample.py::tests.fixtures.sample_python`

##### python: divergent nodes

- `test_sample.py::test_authenticate_invalid` kind: crg=Test rust=Function
- `test_sample.py::test_authenticate_invalid` is_test: crg=1 rust=0
- `test_sample.py::test_authenticate_valid` kind: crg=Test rust=Function
- `test_sample.py::test_authenticate_valid` is_test: crg=1 rust=0
- `test_sample.py::test_process_request_ok` kind: crg=Test rust=Function
- `test_sample.py::test_process_request_ok` is_test: crg=1 rust=0

##### CALLS: edges only in CRG

- `multi_call_example.py::process_document` -> `get`
- `sample_callback_refs.py::register_keyword_callback` -> `set_defaults`
- `sample_callback_refs.py::trigger_executor` -> `ThreadPoolExecutor`
- `sample_callback_refs.py::trigger_executor` -> `submit`
- `sample_callback_refs.py::trigger_filter` -> `filter`
- `sample_callback_refs.py::trigger_filter` -> `list`
- `sample_callback_refs.py::trigger_map` -> `list`
- `sample_callback_refs.py::trigger_map` -> `map`
- `sample_python.py::AuthService.__init__` -> `__init__`
- `sample_python.py::AuthService.__init__` -> `super`
- `sample_python.py::AuthService.authenticate` -> `sample_python.py::AuthService._validate_token`
- `sample_python.py::BaseService.start` -> `print`
- `sample_python.py::create_auth_service` -> `get`
- `sample_python.py::process_request` -> `sample_python.py::AuthService.authenticate`
- `sample_python.py::wrapper` -> `func`
- `test_sample.py::test_authenticate_invalid` -> `AuthService`
- `test_sample.py::test_authenticate_invalid` -> `authenticate`
- `test_sample.py::test_authenticate_valid` -> `AuthService`
- `test_sample.py::test_authenticate_valid` -> `authenticate`
- `test_sample.py::test_process_request_ok` -> `AuthService`
- `test_sample.py::test_process_request_ok` -> `process_request`

##### CALLS: edges only in Rust

- `sample_python.py::authenticate` -> `sample_python.py::_validate_token`
- `sample_python.py::process_request` -> `sample_python.py::authenticate`
- `test_sample.py::test_authenticate_invalid` -> `sample_python.py::AuthService`
- `test_sample.py::test_authenticate_invalid` -> `sample_python.py::authenticate`
- `test_sample.py::test_authenticate_valid` -> `sample_python.py::AuthService`
- `test_sample.py::test_authenticate_valid` -> `sample_python.py::authenticate`
- `test_sample.py::test_process_request_ok` -> `sample_python.py::AuthService`
- `test_sample.py::test_process_request_ok` -> `sample_python.py::process_request`

##### CONTAINS: edges only in CRG

- `caller_example.py` -> `caller_example.py::setup_and_run`
- `multi_call_example.py` -> `multi_call_example.py::_internal_request`
- `multi_call_example.py` -> `multi_call_example.py::process_document`
- `sample_callback_refs.py` -> `sample_callback_refs.py::executor_callback`
- `sample_callback_refs.py` -> `sample_callback_refs.py::filter_callback`
- `sample_callback_refs.py` -> `sample_callback_refs.py::keyword_callback`
- `sample_callback_refs.py` -> `sample_callback_refs.py::map_callback`
- `sample_callback_refs.py` -> `sample_callback_refs.py::register_keyword_callback`
- `sample_callback_refs.py` -> `sample_callback_refs.py::trigger_executor`
- `sample_callback_refs.py` -> `sample_callback_refs.py::trigger_filter`
- `sample_callback_refs.py` -> `sample_callback_refs.py::trigger_map`
- `sample_python.py` -> `sample_python.py::AuthService`
- `sample_python.py` -> `sample_python.py::BaseService`
- `sample_python.py` -> `sample_python.py::_log_action`
- `sample_python.py` -> `sample_python.py::create_auth_service`
- `sample_python.py` -> `sample_python.py::guarded_process`
- `sample_python.py` -> `sample_python.py::process_request`
- `sample_python.py` -> `sample_python.py::wrapper`
- `sample_python.py::AuthService` -> `sample_python.py::AuthService.__init__`
- `sample_python.py::AuthService` -> `sample_python.py::AuthService._validate_token`
- `sample_python.py::AuthService` -> `sample_python.py::AuthService.authenticate`
- `sample_python.py::BaseService` -> `sample_python.py::BaseService.__init__`
- `sample_python.py::BaseService` -> `sample_python.py::BaseService.start`
- `test_sample.py` -> `test_sample.py::test_authenticate_invalid`
- `test_sample.py` -> `test_sample.py::test_authenticate_valid`
- `test_sample.py` -> `test_sample.py::test_process_request_ok`

##### IMPORTS_FROM: edges only in CRG

- `caller_example.py` -> `sample_python.py`
- `sample_callback_refs.py` -> `concurrent.futures`
- `sample_python.py` -> `os`
- `sample_python.py` -> `pathlib`
- `test_sample.py` -> `tests.fixtures.sample_python`

##### IMPORTS_FROM: edges only in Rust

- `caller_example.py::sample_python` -> `caller_example.py::sample_python`
- `sample_callback_refs.py::concurrent.futures` -> `sample_callback_refs.py::concurrent.futures`
- `sample_python.py::os` -> `sample_python.py::os`
- `sample_python.py::pathlib` -> `sample_python.py::pathlib`
- `test_sample.py::tests.fixtures.sample_python` -> `test_sample.py::tests.fixtures.sample_python`

##### INHERITS: edges only in CRG

- `sample_python.py::AuthService` -> `BaseService`

##### REFERENCES: edges only in CRG

- `sample_callback_refs.py::register_keyword_callback` -> `sample_callback_refs.py::keyword_callback`
- `sample_callback_refs.py::trigger_executor` -> `sample_callback_refs.py::executor_callback`
- `sample_callback_refs.py::trigger_filter` -> `sample_callback_refs.py::filter_callback`
- `sample_callback_refs.py::trigger_map` -> `sample_callback_refs.py::map_callback`
- `sample_python.py` -> `sample_python.py::BaseService`

##### TESTED_BY: edges only in CRG

- `AuthService` -> `test_sample.py::test_authenticate_invalid`
- `AuthService` -> `test_sample.py::test_authenticate_valid`
- `AuthService` -> `test_sample.py::test_process_request_ok`
- `authenticate` -> `test_sample.py::test_authenticate_invalid`
- `authenticate` -> `test_sample.py::test_authenticate_valid`
- `process_request` -> `test_sample.py::test_process_request_ok`

### Language corpus: typescript

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| typescript | 16 | 8 | 4 | 0.200 | 0 |

### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 11 | 0 | 0 | 0.000 | 0.000 |
| CONTAINS | 13 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 4 | 4 | 0 | 0.000 | 0.000 |
| INHERITS | 1 | 0 | 0 | 0.000 | 1.000 |
| REFERENCES | 7 | 0 | 0 | 0.000 | 0.000 |
| TESTED_BY | 1 | 0 | 0 | 0.000 | 0.000 |

##### typescript: nodes only in CRG

- `alias_importer.ts`
- `sample_typescript.ts`
- `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserRepository.findById`
- `sample_typescript.ts::UserRepository.save`
- `sample_typescript.ts::UserService.createUser`
- `sample_typescript.ts::UserService.getUser`
- `sample_vitest.test.ts`
- `sample_vitest.test.ts::describe:UserService@L3`
- `sample_vitest.test.ts::it:should create a user@L4`
- `sample_vitest.test.ts::it:should find a user by id@L9`
- `sample_vitest.test.ts::test:alternative test syntax@L15`

##### typescript: nodes only in Rust

- `alias_importer.ts::./sample_typescript`
- `alias_importer.ts::@/lib/utils`
- `sample_typescript.ts::express`
- `sample_vitest.test.ts::./sample_typescript`

##### CALLS: edges only in CRG

- `alias_importer.ts::formatUser` -> `cn`
- `sample_typescript.ts::UserRepository.findById` -> `get`
- `sample_typescript.ts::UserRepository.save` -> `set`
- `sample_typescript.ts::UserService.createUser` -> `now`
- `sample_typescript.ts::UserService.createUser` -> `sample_typescript.ts::UserRepository.save`
- `sample_typescript.ts::UserService.getUser` -> `sample_typescript.ts::UserRepository.findById`
- `sample_typescript.ts::handleGetUser` -> `Number`
- `sample_typescript.ts::handleGetUser` -> `json`
- `sample_typescript.ts::handleGetUser` -> `sample_typescript.ts::UserService.getUser`
- `sample_typescript.ts::handleGetUser` -> `status`
- `sample_vitest.test.ts::it:should find a user by id@L9` -> `sample_typescript.ts::UserRepository.findById`

##### CONTAINS: edges only in CRG

- `alias_importer.ts` -> `alias_importer.ts::formatUser`
- `sample_typescript.ts` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts` -> `sample_typescript.ts::UserRepository`
- `sample_typescript.ts` -> `sample_typescript.ts::UserService`
- `sample_typescript.ts` -> `sample_typescript.ts::handleGetUser`
- `sample_typescript.ts::UserRepository` -> `sample_typescript.ts::UserRepository.findById`
- `sample_typescript.ts::UserRepository` -> `sample_typescript.ts::UserRepository.save`
- `sample_typescript.ts::UserService` -> `sample_typescript.ts::UserService.createUser`
- `sample_typescript.ts::UserService` -> `sample_typescript.ts::UserService.getUser`
- `sample_vitest.test.ts` -> `sample_vitest.test.ts::describe:UserService@L3`
- `sample_vitest.test.ts::describe:UserService@L3` -> `sample_vitest.test.ts::it:should create a user@L4`
- `sample_vitest.test.ts::describe:UserService@L3` -> `sample_vitest.test.ts::it:should find a user by id@L9`
- `sample_vitest.test.ts::describe:UserService@L3` -> `sample_vitest.test.ts::test:alternative test syntax@L15`

##### IMPORTS_FROM: edges only in CRG

- `alias_importer.ts` -> `@/lib/utils`
- `alias_importer.ts` -> `sample_typescript.ts`
- `sample_typescript.ts` -> `express`
- `sample_vitest.test.ts` -> `sample_typescript.ts`

##### IMPORTS_FROM: edges only in Rust

- `alias_importer.ts::./sample_typescript` -> `alias_importer.ts::./sample_typescript`
- `alias_importer.ts::@/lib/utils` -> `alias_importer.ts::@/lib/utils`
- `sample_typescript.ts::express` -> `sample_typescript.ts::express`
- `sample_vitest.test.ts::./sample_typescript` -> `sample_vitest.test.ts::./sample_typescript`

##### INHERITS: edges only in CRG

- `sample_typescript.ts::UserService` -> `UserRepository`

##### REFERENCES: edges only in CRG

- `sample_typescript.ts::UserRepository` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserRepository.findById` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserRepository.save` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserService.createUser` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::UserService.getUser` -> `sample_typescript.ts::UserData`
- `sample_typescript.ts::handleGetUser` -> `Request`
- `sample_typescript.ts::handleGetUser` -> `Response`

##### TESTED_BY: edges only in CRG

- `sample_typescript.ts::UserRepository.findById` -> `sample_vitest.test.ts::it:should find a user by id@L9`

### Language corpus: go

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| go | 45 | 45 | 21 | 0.304 | 1 |

### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 34 | 9 | 6 | 0.162 | 0.353 |
| CONTAINS | 43 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 2 | 0 | 0 | 0.000 | 1.000 |

##### go: nodes only in CRG

- `sample_dead_guard.go`
- `sample_go.go`
- `sample_go.go::InMemoryRepo.FindByID`
- `sample_go.go::InMemoryRepo.Save`
- `sample_go.go::InMemoryRepo.SaveAndReturn`
- `sample_go.go::ShadowA.CallsAfterShadowScope`
- `sample_go.go::ShadowA.CallsBlockShadowedReceiver`
- `sample_go.go::ShadowA.CallsConstShadowedReceiver`
- `sample_go.go::ShadowA.CallsExpressionCaseShadowedReceiver`
- `sample_go.go::ShadowA.CallsForClauseShadowedReceiver`
- `sample_go.go::ShadowA.CallsInitializerScope`
- `sample_go.go::ShadowA.CallsNamedResultShadowedReceiver`
- `sample_go.go::ShadowA.CallsRangeShadowedReceiver`
- `sample_go.go::ShadowA.CallsSameScopeRedeclaration`
- `sample_go.go::ShadowA.CallsSelectCaseShadowedReceiver`
- `sample_go.go::ShadowA.CallsSelectReceiveShadowedReceiver`
- `sample_go.go::ShadowA.CallsShadowedReceiver`
- `sample_go.go::ShadowA.CallsTypeShadowedReceiver`
- `sample_go.go::ShadowA.CallsTypeSwitchInitShadowedReceiver`
- `sample_go.go::ShadowA.CallsTypeSwitchShadowedReceiver`
- `sample_go.go::ShadowA.CallsVarShadowedReceiver`
- `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowB.Save`
- `sample_go.go::ShadowC.Save`

##### go: nodes only in Rust

- `sample_go.go::CallsAfterShadowScope`
- `sample_go.go::CallsBlockShadowedReceiver`
- `sample_go.go::CallsConstShadowedReceiver`
- `sample_go.go::CallsExpressionCaseShadowedReceiver`
- `sample_go.go::CallsForClauseShadowedReceiver`
- `sample_go.go::CallsInitializerScope`
- `sample_go.go::CallsNamedResultShadowedReceiver`
- `sample_go.go::CallsRangeShadowedReceiver`
- `sample_go.go::CallsSameScopeRedeclaration`
- `sample_go.go::CallsSelectCaseShadowedReceiver`
- `sample_go.go::CallsSelectReceiveShadowedReceiver`
- `sample_go.go::CallsShadowedReceiver`
- `sample_go.go::CallsTypeShadowedReceiver`
- `sample_go.go::CallsTypeSwitchInitShadowedReceiver`
- `sample_go.go::CallsTypeSwitchShadowedReceiver`
- `sample_go.go::CallsVarShadowedReceiver`
- `sample_go.go::FindByID`
- `sample_go.go::Save`
- `sample_go.go::Save@L49`
- `sample_go.go::Save@L50`
- `sample_go.go::Save@L51`
- `sample_go.go::SaveAndReturn`
- `sample_go.go::import errors`
- `sample_go.go::import fmt`

##### go: divergent nodes

- `sample_go.go::ShadowC` kind: crg=Class rust=Variable

##### CALLS: edges only in CRG

- `sample_go.go::CreateUser` -> `Save`
- `sample_go.go::InMemoryRepo.FindByID` -> `New`
- `sample_go.go::InMemoryRepo.Save` -> `Printf`
- `sample_go.go::InMemoryRepo.SaveAndReturn` -> `sample_go.go::InMemoryRepo.Save`
- `sample_go.go::NewInMemoryRepo` -> `make`
- `sample_go.go::ShadowA.CallsAfterShadowScope` -> `Save`
- `sample_go.go::ShadowA.CallsAfterShadowScope` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsBlockShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsConstShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsConstShadowedReceiver` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsExpressionCaseShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsForClauseShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsInitializerScope` -> `Save`
- `sample_go.go::ShadowA.CallsInitializerScope` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsNamedResultShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsRangeShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsSameScopeRedeclaration` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsSelectCaseShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsSelectReceiveShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsSelectReceiveShadowedReceiver` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsTypeShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsTypeShadowedReceiver` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsTypeSwitchInitShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsTypeSwitchInitShadowedReceiver` -> `any`
- `sample_go.go::ShadowA.CallsTypeSwitchInitShadowedReceiver` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowA.CallsTypeSwitchShadowedReceiver` -> `Save`
- `sample_go.go::ShadowA.CallsVarShadowedReceiver` -> `Save`

##### CALLS: edges only in Rust

- `sample_dead_guard.go::caller` -> `sample_dead_guard.go::dead_false_call`
- `sample_dead_guard.go::elif_chain` -> `sample_dead_guard.go::dead_in_consequence`
- `sample_dead_guard.go::else_branch` -> `sample_dead_guard.go::dead_in_consequence`

##### CONTAINS: edges only in CRG

- `sample_dead_guard.go` -> `sample_dead_guard.go::caller`
- `sample_dead_guard.go` -> `sample_dead_guard.go::dead_false_call`
- `sample_dead_guard.go` -> `sample_dead_guard.go::dead_in_consequence`
- `sample_dead_guard.go` -> `sample_dead_guard.go::dead_wrapped_func`
- `sample_dead_guard.go` -> `sample_dead_guard.go::elif_chain`
- `sample_dead_guard.go` -> `sample_dead_guard.go::else_branch`
- `sample_dead_guard.go` -> `sample_dead_guard.go::live_final_else`
- `sample_dead_guard.go` -> `sample_dead_guard.go::live_helper`
- `sample_dead_guard.go` -> `sample_dead_guard.go::live_in_else`
- `sample_dead_guard.go` -> `sample_dead_guard.go::live_in_if_true`
- `sample_dead_guard.go` -> `sample_dead_guard.go::live_in_wrapped`
- `sample_dead_guard.go` -> `sample_dead_guard.go::some_condition`
- `sample_dead_guard.go` -> `sample_dead_guard.go::true_guard`
- `sample_go.go` -> `sample_go.go::CreateUser`
- `sample_go.go` -> `sample_go.go::InMemoryRepo`
- `sample_go.go` -> `sample_go.go::NewInMemoryRepo`
- `sample_go.go` -> `sample_go.go::ShadowA`
- `sample_go.go` -> `sample_go.go::ShadowB`
- `sample_go.go` -> `sample_go.go::ShadowC`
- `sample_go.go` -> `sample_go.go::User`
- `sample_go.go` -> `sample_go.go::UserRepository`
- `sample_go.go::InMemoryRepo` -> `sample_go.go::InMemoryRepo.FindByID`
- `sample_go.go::InMemoryRepo` -> `sample_go.go::InMemoryRepo.Save`
- `sample_go.go::InMemoryRepo` -> `sample_go.go::InMemoryRepo.SaveAndReturn`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsAfterShadowScope`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsBlockShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsConstShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsExpressionCaseShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsForClauseShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsInitializerScope`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsNamedResultShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsRangeShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsSameScopeRedeclaration`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsSelectCaseShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsSelectReceiveShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsTypeShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsTypeSwitchInitShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsTypeSwitchShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.CallsVarShadowedReceiver`
- `sample_go.go::ShadowA` -> `sample_go.go::ShadowA.Save`
- `sample_go.go::ShadowB` -> `sample_go.go::ShadowB.Save`
- `sample_go.go::ShadowC` -> `sample_go.go::ShadowC.Save`

##### IMPORTS_FROM: edges only in CRG

- `sample_go.go` -> `errors`
- `sample_go.go` -> `fmt`

### Language corpus: rust

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| rust | 30 | 35 | 18 | 0.383 | 3 |

### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 20 | 15 | 5 | 0.167 | 0.250 |
| CONTAINS | 27 | 0 | 0 | 0.000 | 0.000 |
| IMPLEMENTS | 1 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 1 | 2 | 0 | 0.000 | 0.000 |
| TESTED_BY | 6 | 0 | 0 | 0.000 | 0.000 |

##### rust: nodes only in CRG

- `sample_dead_guard.rs`
- `sample_dead_guard_impl.rs`
- `sample_dead_guard_impl.rs::S.run`
- `sample_rust.rs`
- `sample_rust.rs::InMemoryRepo.find_by_id`
- `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::InMemoryRepo.save`
- `sample_rust.rs::Repository.find_by_id`
- `sample_rust.rs::Repository.save`
- `sample_rust.rs::async_test_is_detected`
- `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs::new_repo_is_empty`

##### rust: nodes only in Rust

- `sample_dead_guard_impl.rs::S@L5`
- `sample_dead_guard_impl.rs::impl<S>::run`
- `sample_rust.rs::InMemoryRepo@L19`
- `sample_rust.rs::InMemoryRepo@L27`
- `sample_rust.rs::impl<InMemoryRepo>::new`
- `sample_rust.rs::impl<Repository for InMemoryRepo>::find_by_id`
- `sample_rust.rs::impl<Repository for InMemoryRepo>::save`
- `sample_rust.rs::println`
- `sample_rust.rs::std::collections::HashMap`
- `sample_rust.rs::tests`
- `sample_rust.rs::tests::assert`
- `sample_rust.rs::tests::assert@L55`
- `sample_rust.rs::tests::assert_eq`
- `sample_rust.rs::tests::async_test_is_detected`
- `sample_rust.rs::tests::create_user_saves_to_repo`
- `sample_rust.rs::tests::new_repo_is_empty`
- `sample_rust.rs::tests::super::*`

##### rust: divergent nodes

- `sample_dead_guard_impl.rs::S` kind: crg=Class rust=impl
- `sample_dead_guard_impl.rs::S` line_start: crg=5 rust=11
- `sample_dead_guard_impl.rs::S` line_end: crg=5 rust=15

##### CALLS: edges only in CRG

- `sample_dead_guard_impl.rs::S.run` -> `sample_dead_guard_impl.rs::live_helper`
- `sample_dead_guard_impl.rs::main` -> `sample_dead_guard_impl.rs::S.run`
- `sample_rust.rs::InMemoryRepo.find_by_id` -> `get`
- `sample_rust.rs::InMemoryRepo.new` -> `HashMap::new`
- `sample_rust.rs::InMemoryRepo.save` -> `insert`
- `sample_rust.rs::InMemoryRepo.save` -> `println`
- `sample_rust.rs::async_test_is_detected` -> `assert`
- `sample_rust.rs::create_user` -> `clone`
- `sample_rust.rs::create_user` -> `save`
- `sample_rust.rs::create_user` -> `to_string`
- `sample_rust.rs::create_user_saves_to_repo` -> `assert_eq`
- `sample_rust.rs::create_user_saves_to_repo` -> `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::create_user_saves_to_repo` -> `sample_rust.rs::create_user`
- `sample_rust.rs::new_repo_is_empty` -> `assert`
- `sample_rust.rs::new_repo_is_empty` -> `sample_rust.rs::InMemoryRepo.new`

##### CALLS: edges only in Rust

- `sample_dead_guard.rs::caller` -> `sample_dead_guard.rs::dead_false_call`
- `sample_dead_guard.rs::elif_chain` -> `sample_dead_guard.rs::dead_in_consequence`
- `sample_dead_guard.rs::else_branch` -> `sample_dead_guard.rs::dead_in_consequence`
- `sample_dead_guard_impl.rs::impl<S>::run` -> `sample_dead_guard_impl.rs::impl<S>::run`
- `sample_dead_guard_impl.rs::impl<S>::run` -> `sample_dead_guard_impl.rs::live_helper`
- `sample_rust.rs::create_user` -> `sample_rust.rs::impl<Repository for InMemoryRepo>::save`
- `sample_rust.rs::impl<InMemoryRepo>::new` -> `sample_rust.rs::impl<InMemoryRepo>::new`
- `sample_rust.rs::tests::create_user_saves_to_repo` -> `sample_rust.rs::create_user`
- `sample_rust.rs::tests::create_user_saves_to_repo` -> `sample_rust.rs::impl<InMemoryRepo>::new`
- `sample_rust.rs::tests::new_repo_is_empty` -> `sample_rust.rs::impl<InMemoryRepo>::new`

##### CONTAINS: edges only in CRG

- `sample_dead_guard.rs` -> `sample_dead_guard.rs::caller`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::dead_false_call`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::dead_in_consequence`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::elif_chain`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::else_branch`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::live_final_else`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::live_helper`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::live_in_else`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::live_in_if_true`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::some_condition`
- `sample_dead_guard.rs` -> `sample_dead_guard.rs::true_guard`
- `sample_dead_guard_impl.rs` -> `sample_dead_guard_impl.rs::S`
- `sample_dead_guard_impl.rs` -> `sample_dead_guard_impl.rs::live_helper`
- `sample_dead_guard_impl.rs` -> `sample_dead_guard_impl.rs::main`
- `sample_dead_guard_impl.rs::S` -> `sample_dead_guard_impl.rs::S.run`
- `sample_rust.rs` -> `sample_rust.rs::InMemoryRepo`
- `sample_rust.rs` -> `sample_rust.rs::Repository`
- `sample_rust.rs` -> `sample_rust.rs::User`
- `sample_rust.rs` -> `sample_rust.rs::async_test_is_detected`
- `sample_rust.rs` -> `sample_rust.rs::create_user`
- `sample_rust.rs` -> `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs` -> `sample_rust.rs::new_repo_is_empty`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.find_by_id`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.save`
- `sample_rust.rs::Repository` -> `sample_rust.rs::Repository.find_by_id`
- `sample_rust.rs::Repository` -> `sample_rust.rs::Repository.save`

##### IMPLEMENTS: edges only in CRG

- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::Repository`

##### IMPORTS_FROM: edges only in CRG

- `sample_rust.rs` -> `std::collections::HashMap`

##### IMPORTS_FROM: edges only in Rust

- `sample_rust.rs::std::collections::HashMap` -> `sample_rust.rs::std::collections::HashMap`
- `sample_rust.rs::tests::super::*` -> `sample_rust.rs::tests::super::*`

##### TESTED_BY: edges only in CRG

- `assert` -> `sample_rust.rs::async_test_is_detected`
- `assert` -> `sample_rust.rs::new_repo_is_empty`
- `assert_eq` -> `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs::InMemoryRepo.new` -> `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs::InMemoryRepo.new` -> `sample_rust.rs::new_repo_is_empty`
- `sample_rust.rs::create_user` -> `sample_rust.rs::create_user_saves_to_repo`

### Language corpus: java

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| java | 36 | 47 | 12 | 0.169 | 9 |

### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 15 | 3 | 0 | 0.000 | 0.000 |
| CONTAINS | 34 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 11 | 6 | 0 | 0.000 | 0.000 |
| INHERITS | 3 | 0 | 0 | 0.000 | 1.000 |
| INJECTS | 4 | 0 | 0 | 0.000 | 1.000 |

##### java: nodes only in CRG

- `SampleJava.java`
- `SampleJava.java::CachedRepo.save`
- `SampleJava.java::InMemoryRepo.findById`
- `SampleJava.java::InMemoryRepo.save`
- `SampleJava.java::User.User`
- `SampleJava.java::User.getEmail`
- `SampleJava.java::User.getId`
- `SampleJava.java::User.getName`
- `SampleJava.java::UserRepository.findById`
- `SampleJava.java::UserRepository.save`
- `SampleJava.java::UserService.UserService`
- `SampleJava.java::UserService.createUser`
- `SampleJava.java::UserService.getUser`
- `SpringDI.java`
- `SpringDI.java::AppConfig.orderRepository`
- `SpringDI.java::AuditLogger.AuditLogger`
- `SpringDI.java::AuditLogger.log`
- `SpringDI.java::JpaOrderRepository.findById`
- `SpringDI.java::JpaOrderRepository.save`
- `SpringDI.java::NotificationService.notify`
- `SpringDI.java::Order.getId`
- `SpringDI.java::OrderRepository.findById`
- `SpringDI.java::OrderRepository.save`
- `SpringDI.java::OrderService.placeOrder`

##### java: nodes only in Rust

- `SampleJava.java::User@L12`
- `SampleJava.java::UserService@L43`
- `SampleJava.java::com.example.auth`
- `SampleJava.java::createUser`
- `SampleJava.java::findById`
- `SampleJava.java::findById@L31`
- `SampleJava.java::getEmail`
- `SampleJava.java::getId`
- `SampleJava.java::getName`
- `SampleJava.java::getUser`
- `SampleJava.java::import java.util.HashMap`
- `SampleJava.java::import java.util.Map`
- `SampleJava.java::import java.util.Optional`
- `SampleJava.java::save`
- `SampleJava.java::save@L36`
- `SampleJava.java::save@L9`
- `SpringDI.java::AuditLogger@L54`
- `SpringDI.java::com.example.shop`
- `SpringDI.java::findById`
- `SpringDI.java::findById@L15`
- `SpringDI.java::getId`
- `SpringDI.java::import lombok.RequiredArgsConstructor`
- `SpringDI.java::import org.springframework.beans.factory.annotation.Autowired`
- `SpringDI.java::import org.springframework.context.annotation.Bean`
- `SpringDI.java::import org.springframework.context.annotation.Configuration`
- `SpringDI.java::import org.springframework.stereotype.Component`
- `SpringDI.java::import org.springframework.stereotype.Controller`
- `SpringDI.java::import org.springframework.stereotype.Repository`
- `SpringDI.java::import org.springframework.stereotype.Service`
- `SpringDI.java::log`
- `SpringDI.java::notify`
- `SpringDI.java::orderRepository`
- `SpringDI.java::placeOrder`
- `SpringDI.java::save`
- `SpringDI.java::save@L21`

##### java: divergent nodes

- `SampleJava.java::User` kind: crg=Class rust=Function
- `SampleJava.java::User` line_start: crg=12 rust=17
- `SampleJava.java::User` line_end: crg=26 rust=21
- `SampleJava.java::UserService` kind: crg=Class rust=Function
- `SampleJava.java::UserService` line_start: crg=43 rust=46
- `SampleJava.java::UserService` line_end: crg=59 rust=48
- `SpringDI.java::AuditLogger` kind: crg=Class rust=Function
- `SpringDI.java::AuditLogger` line_start: crg=54 rust=58
- `SpringDI.java::AuditLogger` line_end: crg=64 rust=61

##### CALLS: edges only in CRG

- `SampleJava.java` -> `HashMap`
- `SampleJava.java::CachedRepo.save` -> `SampleJava.java::UserRepository.save`
- `SampleJava.java::InMemoryRepo.findById` -> `get`
- `SampleJava.java::InMemoryRepo.findById` -> `ofNullable`
- `SampleJava.java::InMemoryRepo.save` -> `SampleJava.java::User.getId`
- `SampleJava.java::InMemoryRepo.save` -> `println`
- `SampleJava.java::InMemoryRepo.save` -> `put`
- `SampleJava.java::UserService.createUser` -> `SampleJava.java::User`
- `SampleJava.java::UserService.createUser` -> `SampleJava.java::UserRepository.save`
- `SampleJava.java::UserService.getUser` -> `SampleJava.java::UserRepository.findById`
- `SpringDI.java::AppConfig.orderRepository` -> `SpringDI.java::JpaOrderRepository`
- `SpringDI.java::NotificationService.notify` -> `SpringDI.java::JpaOrderRepository.findById`
- `SpringDI.java::OrderService.placeOrder` -> `SpringDI.java::JpaOrderRepository.save`
- `SpringDI.java::OrderService.placeOrder` -> `SpringDI.java::NotificationService.notify`
- `SpringDI.java::OrderService.placeOrder` -> `SpringDI.java::Order.getId`

##### CALLS: edges only in Rust

- `SampleJava.java::save@L36` -> `SampleJava.java::getId`
- `SpringDI.java::placeOrder` -> `SpringDI.java::getId`
- `SpringDI.java::placeOrder` -> `SpringDI.java::notify`

##### CONTAINS: edges only in CRG

- `SampleJava.java` -> `SampleJava.java::CachedRepo`
- `SampleJava.java` -> `SampleJava.java::InMemoryRepo`
- `SampleJava.java` -> `SampleJava.java::User`
- `SampleJava.java` -> `SampleJava.java::UserRepository`
- `SampleJava.java` -> `SampleJava.java::UserService`
- `SampleJava.java::CachedRepo` -> `SampleJava.java::CachedRepo.save`
- `SampleJava.java::InMemoryRepo` -> `SampleJava.java::InMemoryRepo.findById`
- `SampleJava.java::InMemoryRepo` -> `SampleJava.java::InMemoryRepo.save`
- `SampleJava.java::User` -> `SampleJava.java::User.User`
- `SampleJava.java::User` -> `SampleJava.java::User.getEmail`
- `SampleJava.java::User` -> `SampleJava.java::User.getId`
- `SampleJava.java::User` -> `SampleJava.java::User.getName`
- `SampleJava.java::UserRepository` -> `SampleJava.java::UserRepository.findById`
- `SampleJava.java::UserRepository` -> `SampleJava.java::UserRepository.save`
- `SampleJava.java::UserService` -> `SampleJava.java::UserService.UserService`
- `SampleJava.java::UserService` -> `SampleJava.java::UserService.createUser`
- `SampleJava.java::UserService` -> `SampleJava.java::UserService.getUser`
- `SpringDI.java` -> `SpringDI.java::AppConfig`
- `SpringDI.java` -> `SpringDI.java::AuditLogger`
- `SpringDI.java` -> `SpringDI.java::JpaOrderRepository`
- `SpringDI.java` -> `SpringDI.java::NotificationService`
- `SpringDI.java` -> `SpringDI.java::Order`
- `SpringDI.java` -> `SpringDI.java::OrderRepository`
- `SpringDI.java` -> `SpringDI.java::OrderService`
- `SpringDI.java::AppConfig` -> `SpringDI.java::AppConfig.orderRepository`
- `SpringDI.java::AuditLogger` -> `SpringDI.java::AuditLogger.AuditLogger`
- `SpringDI.java::AuditLogger` -> `SpringDI.java::AuditLogger.log`
- `SpringDI.java::JpaOrderRepository` -> `SpringDI.java::JpaOrderRepository.findById`
- `SpringDI.java::JpaOrderRepository` -> `SpringDI.java::JpaOrderRepository.save`
- `SpringDI.java::NotificationService` -> `SpringDI.java::NotificationService.notify`
- `SpringDI.java::Order` -> `SpringDI.java::Order.getId`
- `SpringDI.java::OrderRepository` -> `SpringDI.java::OrderRepository.findById`
- `SpringDI.java::OrderRepository` -> `SpringDI.java::OrderRepository.save`
- `SpringDI.java::OrderService` -> `SpringDI.java::OrderService.placeOrder`

##### IMPORTS_FROM: edges only in CRG

- `SampleJava.java` -> `java.util.HashMap`
- `SampleJava.java` -> `java.util.Map`
- `SampleJava.java` -> `java.util.Optional`
- `SpringDI.java` -> `lombok.RequiredArgsConstructor`
- `SpringDI.java` -> `org.springframework.beans.factory.annotation.Autowired`
- `SpringDI.java` -> `org.springframework.context.annotation.Bean`
- `SpringDI.java` -> `org.springframework.context.annotation.Configuration`
- `SpringDI.java` -> `org.springframework.stereotype.Component`
- `SpringDI.java` -> `org.springframework.stereotype.Controller`
- `SpringDI.java` -> `org.springframework.stereotype.Repository`
- `SpringDI.java` -> `org.springframework.stereotype.Service`

##### IMPORTS_FROM: edges only in Rust

- `SpringDI.java::import org.springframework.beans.factory.annotation.Autowired` -> `SpringDI.java::import org.springframework.beans.factory.annotation.Autowired`
- `SpringDI.java::import org.springframework.context.annotation.Bean` -> `SpringDI.java::import org.springframework.context.annotation.Bean`
- `SpringDI.java::import org.springframework.context.annotation.Configuration` -> `SpringDI.java::import org.springframework.context.annotation.Configuration`
- `SpringDI.java::import org.springframework.stereotype.Component` -> `SpringDI.java::import org.springframework.stereotype.Component`
- `SpringDI.java::import org.springframework.stereotype.Controller` -> `SpringDI.java::import org.springframework.stereotype.Controller`
- `SpringDI.java::import org.springframework.stereotype.Repository` -> `SpringDI.java::import org.springframework.stereotype.Repository`

##### INHERITS: edges only in CRG

- `SampleJava.java::CachedRepo` -> `InMemoryRepo`
- `SampleJava.java::InMemoryRepo` -> `UserRepository`
- `SpringDI.java::JpaOrderRepository` -> `OrderRepository`

##### INJECTS: edges only in CRG

- `SpringDI.java::AuditLogger` -> `OrderRepository`
- `SpringDI.java::NotificationService` -> `OrderRepository`
- `SpringDI.java::OrderService` -> `NotificationService`
- `SpringDI.java::OrderService` -> `OrderRepository`

## Rust imports probe (antes)

Sonda `scripts/parity/probes/rust-imports` (workspace `app` + `util-lib`: `crate::`, `self::`,
`super::`, crate externa, primeiro segmento sem ancora, tipo de outro arquivo, `impl Tr for`,
`std::fmt`/`Vec::new()`, modulos `Glob`/`glob`), antes da resolucao de modulos Rust entre
arquivos. CRG em `bbdda4b`. Nos 22/22 (1.000); CONTAINS 0.737 (so a hierarquia de diretorios
existe apenas no Rust); CALLS 0.267 (resolvido 0.429); IMPORTS_FROM 0.500 (resolvido 0.667);
IMPLEMENTS 1.000; TESTED_BY n/a (sem testes na sonda).

Arestas so no CRG:
- CALLS: `main.rs::main` -> `Glob.rs::upper`, `glob.rs::lower`, `store/disk.rs::Disk.open`,
  `util-lib/src/text.rs::shout`; `store/disk.rs::Disk.open` -> `load` (texto).
- IMPORTS_FROM: `main.rs` -> `store/mod.rs`, `main.rs` -> `util-lib/src/text.rs`.

Grafo inteiro deste repositorio (mesmo commit do CRG, inclui a propria sonda): IMPORTS_FROM
0.729 (resolvido 0.534; crg 1597, rust 1860, both 1458), CALLS 0.931 (resolvido 0.859; crg
18310, rust 18214, both 17614). A Tarefa 7 compara contra estes valores.

## Rust imports probe (depois)

Plano `docs/superpowers/plans/2026-11-24-fase2-rust-modulos.md` (Tarefas 2-6 e os fixes
`3b7ce1c`, `c897a49`, `3840ab1`), CRG em `bbdda4b`, binario debug.

| sonda rust-imports | antes | depois |
|---|---|---|
| nos | 1.000 (22/22) | 1.000 (22/22) |
| CONTAINS | 0.737 | 0.737 |
| CALLS | 0.267 (resolvido 0.429) | 1.000 (resolvido 1.000; 9/9) |
| IMPORTS_FROM | 0.500 (resolvido 0.667) | 1.000 (resolvido 1.000; 7/7) |
| IMPLEMENTS | 1.000 | 1.000 (1/1) |

Nenhuma aresta so de um lado em CALLS, IMPORTS_FROM ou IMPLEMENTS. Os 5 CONTAINS so no Rust sao
a hierarquia de diretorios (`app` -> `app/src`, `app/src` -> `app/src/store`, `util-lib` ->
`util-lib/src` e raiz -> `app`/`util-lib`), que o CRG nao grava. A sonda nao foi alterada:
`use super::config` em `store/disk.rs` nomeia um item inexistente de `store`, e
`config::load()` dali vira `load` com `claimed_endpoint` `store/mod.rs::config.load` nos dois
lados.

Tres defeitos achados na medicao e corrigidos antes destes numeros:
- `3b7ce1c`: com `codegraph index .` as chaves de arquivo sao `./app/src/main.rs`; o layout do
  crate comparava com `app/src/main.rs` e nao achava a raiz, entao nenhum import Rust resolvia
  (no HEAD `523a0be` a sonda dava IMPORTS_FROM 0.071, CALLS 0.188, IMPLEMENTS 0.000, pior que o
  "antes"; os testes unitarios usavam chaves sem `./`).
- `c897a49`: chamada com escopo cujo caminho nomeia um arquivo mas o item nao existe nele: o CRG
  qualifica mesmo assim e o `_settle_unbound_endpoints` grava o nome solto; o Rust deixava
  `config::load`.
- `3840ab1`: o CRG monta o mapa de imports so com os itens de topo do arquivo; o Rust usava
  tambem os `use` de dentro de `mod { }`/corpo de funcao para expandir chamadas (11 arestas
  deste repo). `GRAPH_SHAPE_VERSION` foi a `crg-shape-12`.

Corpus: a linha rust segue 1.000 em nos, CALLS, CONTAINS, IMPLEMENTS, IMPORTS_FROM, TESTED_BY,
flows e ARI. Nas outras linguagens so mudou o ARI de python (0.403 -> 0.909, mesmas 8/5
comunidades; o parser python nao mudou e a causa nao foi atribuida: nao se verificou se
vem de variacao do Leiden de um dos lados, entao indexar duas vezes e comparar `community_id`
fica pendente).

Grafo inteiro deste repositorio (HEAD `3840ab1`, inclui a sonda):

| tipo | antes | depois |
|---|---|---|
| IMPORTS_FROM | 0.729 (resolvido 0.534; crg 1597, rust 1860, both 1458) | 0.991 (resolvido 1.000; crg 1605, rust 1591, both 1591) |
| CALLS | 0.931 (resolvido 0.859; crg 18310, rust 18214, both 17614) | 0.964 (resolvido 0.914; crg 18663, rust 18551, both 18270) |
| IMPLEMENTS | - | 0.891 (resolvido 0.846; crg 124, rust 120, both 115) |

O que resta:
- IMPORTS_FROM: as 14 arestas so no CRG saem de `schema_v15.sql`/`schema_cg.sql` (o Rust nao
  indexa SQL). Entre arquivos Rust/Python a igualdade e total; 271 arestas `File` -> `File` nos
  dois lados. O Rust grava mais linhas repetidas da mesma aresta (1991 linhas para 1591 pares;
  CRG 1682 para 1605).
- IMPLEMENTS so no CRG: `impl From<X> for T` (4, traco generico de fora), `impl CounterExt for
  dashmap::DashMap<..>` (tipo com caminho), `TreeSitterParser` -> `codegraph-core/src/lib.rs::
  CodeParser` (o CRG grava o alvo qualificado mesmo sem no, pois `CodeParser` vem de `pub use
  traits::*`; o Rust deixa `CodeParser`); `Scheduler`/`Aggregator` declarados em `mod` inline de
  `coordination/mod.rs` ficam texto no CRG e o Rust liga ao no do mesmo arquivo (4).
- CALLS (393 so no CRG, 281 so no Rust): a maior parte vem dos 32 nos com sufixo divergente
  (`@L<linha>` no Rust, `:S<hash>` no CRG, funcoes duplicadas por `cfg`) e da resolucao por nome
  solto/lexica (100 chamadas que o CRG deixa texto e o Rust liga; das 393 so no CRG, so 12 o CRG liga e o Rust deixa como texto, o resto tem alvo de no divergente). Ligadas a
  caminhos Rust: 59 `Tipo::m` que o CRG deixa texto e o Rust liga por busca exata (fora deste
  plano), mais os limites de `pub use`, glob e `mod` listados no PLANO.md.

## INHERITS no GraphQuery

Plano `docs/superpowers/plans/2026-11-25-inherits-graphquery.md` (2026-11-25, HEAD `062882b`
para a medicao). So leitura: `Snapshot::load` liga o texto de `INHERITS`/`IMPLEMENTS` ao no de
tipo por nome; o SQLite nao muda. Sonda `scripts/parity/probes/inheritance` (25 arquivos):
por linguagem, uma base com duas subclasses, uma base `Shared` definida em dois arquivos sem
import que desempate (ambigua), bases externas (`unittest.TestCase`, `React.Component`,
`RuntimeException`, `sync.Mutex`, `vendor.Shared`), Java `implements Shape<Integer>` e Python
`class Square(shapes.Shape)`. Scripts `scripts/parity/inherits/crg_inherits.py` e
`compare_inherits.py`: `query_graph inheritors_of` por no de tipo e `get_impact_radius` por
arquivo (profundidade 1 e 2), Jaccard somado (Σ∩/Σ∪) por linguagem.

As arestas `INHERITS` gravadas sao identicas nos dois bancos (15 linhas, mesmo texto).

| linguagem | conjunto | comparados | iguais | Σ∩ | Σ∪ | Jaccard |
|---|---|---|---|---|---|---|
| go | inheritors_of | 7 | 7 | 2 | 2 | 1.000 |
| go | impacto d1 | 6 | 5 | 0 | 2 | 0.000 |
| go | impacto d2 | 6 | 5 | 0 | 2 | 0.000 |
| java | inheritors_of | 7 | 4 | 1 | 4 | 0.250 |
| java | impacto d1 | 7 | 6 | 0 | 2 | 0.000 |
| java | impacto d2 | 7 | 6 | 0 | 2 | 0.000 |
| py | inheritors_of | 7 | 4 | 1 | 4 | 0.250 |
| py | impacto d1 | 6 | 5 | 2 | 4 | 0.500 |
| py | impacto d2 | 6 | 5 | 2 | 5 | 0.400 |
| ts | inheritors_of | 7 | 5 | 2 | 4 | 0.500 |
| ts | impacto d1 | 6 | 5 | 2 | 4 | 0.500 |
| ts | impacto d2 | 6 | 5 | 2 | 4 | 0.500 |

Antes do plano o snapshot descartava todo alvo de texto, entao o impacto do Rust era igual ao
do CRG nesta sonda e `inheritors_of` nao existia. Toda divergencia tem uma de tres causas:

1. **O impacto do CRG nao segue heranca em texto.** `get_impact_radius` do CRG junta arestas
   por `qualified_name`; `INHERITS` com alvo `Shape` nunca casa com `py/shapes.py::Shape`.
   Assim, mudar o arquivo da base nao alcanca as subclasses no CRG; no Rust alcanca (objetivo
   do plano). Em todas as 8 divergencias de impacto o lado so-CRG e vazio e o lado so-Rust e
   exatamente as subclasses (`Circle`, `Square` em cada linguagem) e, na profundidade 2 em
   Python, `CircleCase.test_area` (chama `Circle()`).
2. **Base ambigua.** `Leaf extends Shared` com `Shared` em dois arquivos (py, ts, java): o CRG
   lista `Leaf` em `inheritors_of` dos dois `Shared` com `inferred_by: "bare_name"`; o Rust nao
   cria aresta e reporta `unresolved = 1` nos dois (6 divergencias). Em Go `Leaf` embute
   `shared.Shared` com import `vendor/shared` (fora do repo): os dois lados nao ligam (o CRG
   porque o texto `shared.Shared` nao e o nome `Shared`; o Rust porque os dois `Shared` Go sao
   ambiguos, `unresolved = 1` em cada), entao o conjunto e igual.
3. **Texto que o CRG nao casa por nome exato.** O fallback do CRG compara o texto inteiro do
   alvo com o nome da base: `shapes.Shape` (Python `Square`) e `Shape<Integer>` (Java
   `Circle`) ficam sem ligacao no CRG; o Rust usa o ultimo segmento sem argumentos de tipo e
   liga ambos (2 divergencias, Rust-only).

Nenhum caso so-CRG fora da causa 2. Bases externas nao viram aresta em nenhum dos lados.
`Widget extends React.Component` (TS) nao gera `INHERITS` em nenhum dos dois extratores.

Corpus (`parity.py corpus`) depois do plano: nos e arestas identicos ao "Depois" acima; a unica
linha diferente e o ARI de python (0.909 contra 0.403 em "Depois"), o mesmo valor ja medido
no fechamento da resolucao de modulos Rust (secao anterior); o `AnalysisGraph` nao mudou neste
plano.

Ferramentas afetadas. O `Snapshot` alimenta `get_impact_radius`, `get_review_context` e
`get_minimal_context` (impacto e contagem de `dependents`), `traverse_graph`, `query_graph`,
dependencias diretas/reversas, ciclos, `calculate_coupling_metrics` e o `risk_score` de
`find_complexity_hotspots`; todas passam a seguir a heranca em texto. As que usam o
`AnalysisGraph` nao mudam: `detect_changes` (`risk_score`/`risk_signals`,
`review_priorities`, flows afetados), o `risk_score` da revisao, flows, comunidades e as
ferramentas `get_hub_nodes`/`get_bridge_nodes`/`get_knowledge_gaps`/
`get_surprising_connections`/`get_suggested_questions`. O metodo de biblioteca
`GraphQuery::get_hub_nodes` conta heranca, mas nenhuma ferramenta o chama. Consequencia: o
impacto do Rust e um superconjunto do impacto do CRG quando um arquivo alterado define uma base
com subclasses no repo. Isso foi aceito como melhoria intencional. O criterio da Fase 3 agora
exclui essas subclasses/implementacoes. Remedicao em `docs/parity/changes.md`
("Depois de INHERITS no GraphQuery"): cenarios da Fase 3 inalterados; editando `errors.py` do
CRG, o unico no so-Rust e `CorruptGraphDatabaseError(GraphStoreError)`.

Limitacao conhecida (camada (c), repositorio inteiro): uma base importada de fora do repo pode
ser ligada a uma classe homonima sem relacao no repo. Exemplos: Python
`from django.db import Model; class X(Model)` com outro `Model` no repo; Go `shared.Shared` de
`vendor/shared` quando ha um unico `Shared` Go no repo (na sonda ha dois, por isso fica
ambiguo). O import externo nao gera aresta `File -> File`, entao a camada (b) nao decide. O
`inheritors_of` do CRG faz o mesmo pelo fallback de nome solto.
