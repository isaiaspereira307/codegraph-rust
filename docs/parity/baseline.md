# Parity baseline

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

"Depois" abaixo e o relatorio regerado apos o plano Go.

# Depois

CRG commit `bbdda4b`; corpus: fixtures from `tests/fixtures`, one repository per language.

### Language corpus: python

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| python | 31 | 31 | 31 | 1.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 25 | 26 | 18 | 0.545 | 1.000 |
| CONTAINS | 26 | 26 | 26 | 1.000 | 1.000 |
| IMPORTS_FROM | 5 | 5 | 5 | 1.000 | 1.000 |
| INHERITS | 1 | 0 | 0 | 0.000 | 1.000 |
| REFERENCES | 5 | 0 | 0 | 0.000 | 0.000 |
| TESTED_BY | 6 | 6 | 4 | 0.500 | 0.500 |

##### CALLS: edges only in CRG

- `multi_call_example.py::process_document` -> `get`
- `sample_callback_refs.py::register_keyword_callback` -> `set_defaults`
- `sample_callback_refs.py::trigger_executor` -> `submit`
- `sample_python.py::AuthService.__init__` -> `__init__`
- `sample_python.py::create_auth_service` -> `get`
- `test_sample.py::test_authenticate_invalid` -> `authenticate`
- `test_sample.py::test_authenticate_valid` -> `authenticate`

##### CALLS: edges only in Rust

- `multi_call_example.py::process_document` -> `first.get`
- `multi_call_example.py::process_document` -> `second.get`
- `sample_callback_refs.py::register_keyword_callback` -> `parser.set_defaults`
- `sample_callback_refs.py::trigger_executor` -> `executor.submit`
- `sample_python.py::AuthService.__init__` -> `super().__init__`
- `sample_python.py::create_auth_service` -> `os.environ.get`
- `test_sample.py::test_authenticate_invalid` -> `service.authenticate`
- `test_sample.py::test_authenticate_valid` -> `service.authenticate`

##### INHERITS: edges only in CRG

- `sample_python.py::AuthService` -> `BaseService`

##### REFERENCES: edges only in CRG

- `sample_callback_refs.py::register_keyword_callback` -> `sample_callback_refs.py::keyword_callback`
- `sample_callback_refs.py::trigger_executor` -> `sample_callback_refs.py::executor_callback`
- `sample_callback_refs.py::trigger_filter` -> `sample_callback_refs.py::filter_callback`
- `sample_callback_refs.py::trigger_map` -> `sample_callback_refs.py::map_callback`
- `sample_python.py` -> `sample_python.py::BaseService`

##### TESTED_BY: edges only in CRG

- `authenticate` -> `test_sample.py::test_authenticate_invalid`
- `authenticate` -> `test_sample.py::test_authenticate_valid`

##### TESTED_BY: edges only in Rust

- `service.authenticate` -> `test_sample.py::test_authenticate_invalid`
- `service.authenticate` -> `test_sample.py::test_authenticate_valid`

### Language corpus: typescript

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| typescript | 16 | 16 | 16 | 1.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 11 | 11 | 11 | 1.000 | 1.000 |
| CONTAINS | 13 | 13 | 13 | 1.000 | 1.000 |
| IMPORTS_FROM | 4 | 4 | 4 | 1.000 | 1.000 |
| INHERITS | 1 | 1 | 1 | 1.000 | 1.000 |
| REFERENCES | 7 | 0 | 0 | 0.000 | 0.000 |
| TESTED_BY | 1 | 1 | 1 | 1.000 | 1.000 |

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

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 34 | 34 | 34 | 1.000 | 1.000 |
| CONTAINS | 43 | 43 | 43 | 1.000 | 1.000 |
| IMPORTS_FROM | 2 | 2 | 2 | 1.000 | 1.000 |

### Language corpus: rust

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| rust | 30 | 36 | 28 | 0.737 | 9 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 20 | 19 | 13 | 0.500 | 0.562 |
| CONTAINS | 27 | 25 | 21 | 0.677 | 0.677 |
| IMPLEMENTS | 1 | 0 | 0 | 0.000 | 0.000 |
| IMPORTS_FROM | 1 | 2 | 1 | 0.500 | 1.000 |
| TESTED_BY | 6 | 0 | 0 | 0.000 | 0.000 |

##### rust: nodes only in CRG

- `sample_rust.rs::Repository.find_by_id`
- `sample_rust.rs::Repository.save`

##### rust: nodes only in Rust

- `sample_dead_guard_impl.rs::S@L11`
- `sample_rust.rs::InMemoryRepo@L15`
- `sample_rust.rs::InMemoryRepo@L27`
- `sample_rust.rs::println`
- `sample_rust.rs::tests`
- `sample_rust.rs::tests::assert`
- `sample_rust.rs::tests::assert@L55`
- `sample_rust.rs::tests::assert_eq`

##### rust: divergent nodes

- `sample_rust.rs::InMemoryRepo` kind: crg=Class rust=impl
- `sample_rust.rs::InMemoryRepo` line_start: crg=15 rust=19
- `sample_rust.rs::InMemoryRepo` line_end: crg=17 rust=25
- `sample_rust.rs::async_test_is_detected` kind: crg=Test rust=Function
- `sample_rust.rs::async_test_is_detected` is_test: crg=1 rust=0
- `sample_rust.rs::create_user_saves_to_repo` kind: crg=Test rust=Function
- `sample_rust.rs::create_user_saves_to_repo` is_test: crg=1 rust=0
- `sample_rust.rs::new_repo_is_empty` kind: crg=Test rust=Function
- `sample_rust.rs::new_repo_is_empty` is_test: crg=1 rust=0

##### CALLS: edges only in CRG

- `sample_dead_guard_impl.rs::main` -> `sample_dead_guard_impl.rs::S.run`
- `sample_rust.rs::InMemoryRepo.new` -> `HashMap::new`
- `sample_rust.rs::InMemoryRepo.save` -> `println`
- `sample_rust.rs::async_test_is_detected` -> `assert`
- `sample_rust.rs::create_user` -> `save`
- `sample_rust.rs::create_user_saves_to_repo` -> `assert_eq`
- `sample_rust.rs::new_repo_is_empty` -> `assert`

##### CALLS: edges only in Rust

- `sample_dead_guard.rs::caller` -> `sample_dead_guard.rs::dead_false_call`
- `sample_dead_guard.rs::elif_chain` -> `sample_dead_guard.rs::dead_in_consequence`
- `sample_dead_guard.rs::else_branch` -> `sample_dead_guard.rs::dead_in_consequence`
- `sample_dead_guard_impl.rs::S.run` -> `sample_dead_guard_impl.rs::S.run`
- `sample_rust.rs::InMemoryRepo.new` -> `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::create_user` -> `sample_rust.rs::InMemoryRepo.save`

##### CONTAINS: edges only in CRG

- `sample_rust.rs` -> `sample_rust.rs::InMemoryRepo`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.find_by_id`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::InMemoryRepo.save`
- `sample_rust.rs::Repository` -> `sample_rust.rs::Repository.find_by_id`
- `sample_rust.rs::Repository` -> `sample_rust.rs::Repository.save`

##### CONTAINS: edges only in Rust

- `sample_rust.rs` -> `sample_rust.rs::InMemoryRepo@L15`
- `sample_rust.rs::InMemoryRepo@L15` -> `sample_rust.rs::InMemoryRepo.find_by_id`
- `sample_rust.rs::InMemoryRepo@L15` -> `sample_rust.rs::InMemoryRepo.new`
- `sample_rust.rs::InMemoryRepo@L15` -> `sample_rust.rs::InMemoryRepo.save`

##### IMPLEMENTS: edges only in CRG

- `sample_rust.rs::InMemoryRepo` -> `sample_rust.rs::Repository`

##### IMPORTS_FROM: edges only in Rust

- `sample_rust.rs` -> `super::*`

##### TESTED_BY: edges only in CRG

- `assert` -> `sample_rust.rs::async_test_is_detected`
- `assert` -> `sample_rust.rs::new_repo_is_empty`
- `assert_eq` -> `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs::InMemoryRepo.new` -> `sample_rust.rs::create_user_saves_to_repo`
- `sample_rust.rs::InMemoryRepo.new` -> `sample_rust.rs::new_repo_is_empty`
- `sample_rust.rs::create_user` -> `sample_rust.rs::create_user_saves_to_repo`

### Language corpus: java

#### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| java | 36 | 38 | 36 | 0.947 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard |
|---|---|---|---|---|---|
| CALLS | 15 | 12 | 7 | 0.350 | 0.300 |
| CONTAINS | 34 | 34 | 34 | 1.000 | 1.000 |
| IMPORTS_FROM | 11 | 11 | 11 | 1.000 | 1.000 |
| INHERITS | 3 | 0 | 0 | 0.000 | 1.000 |
| INJECTS | 4 | 0 | 0 | 0.000 | 1.000 |

##### java: nodes only in Rust

- `SampleJava.java::com.example.auth`
- `SpringDI.java::com.example.shop`

##### CALLS: edges only in CRG

- `SampleJava.java` -> `HashMap`
- `SampleJava.java::CachedRepo.save` -> `SampleJava.java::UserRepository.save`
- `SampleJava.java::UserService.createUser` -> `SampleJava.java::User`
- `SampleJava.java::UserService.createUser` -> `SampleJava.java::UserRepository.save`
- `SampleJava.java::UserService.getUser` -> `SampleJava.java::UserRepository.findById`
- `SpringDI.java::AppConfig.orderRepository` -> `SpringDI.java::JpaOrderRepository`
- `SpringDI.java::NotificationService.notify` -> `SpringDI.java::JpaOrderRepository.findById`
- `SpringDI.java::OrderService.placeOrder` -> `SpringDI.java::JpaOrderRepository.save`

##### CALLS: edges only in Rust

- `SampleJava.java::CachedRepo.save` -> `save`
- `SampleJava.java::UserService.createUser` -> `save`
- `SampleJava.java::UserService.getUser` -> `findById`
- `SpringDI.java::NotificationService.notify` -> `findById`
- `SpringDI.java::OrderService.placeOrder` -> `save`

##### INHERITS: edges only in CRG

- `SampleJava.java::CachedRepo` -> `InMemoryRepo`
- `SampleJava.java::InMemoryRepo` -> `UserRepository`
- `SpringDI.java::JpaOrderRepository` -> `OrderRepository`

##### INJECTS: edges only in CRG

- `SpringDI.java::AuditLogger` -> `OrderRepository`
- `SpringDI.java::NotificationService` -> `OrderRepository`
- `SpringDI.java::OrderService` -> `NotificationService`
- `SpringDI.java::OrderService` -> `OrderRepository`

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
