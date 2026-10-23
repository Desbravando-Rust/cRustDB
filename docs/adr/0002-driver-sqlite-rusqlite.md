# Driver SQLite com rusqlite em thread dedicada

* **Status:** Aceito
* **Data:** 2026-10-22
* **Issue Relacionada:** #26

## Contexto e Problema
A v1 suporta só PostgreSQL e MySQL/MariaDB (spec §10). A issue #26 adiciona SQLite. As diferenças em relação aos drivers de rede são estas:

* A conexão é um arquivo local. Não há host, porta, usuário, TLS nem túnel SSH.
* Não existem schemas, funções, procedures nem sequences.
* `EXPLAIN QUERY PLAN` não estima linhas.
* O cancelamento usa `sqlite3_interrupt`, sem cancel no servidor.
* A API C é síncrona e bloqueante. Um resultado pausado (`sqlite3_stmt` em andamento) toma emprestada a conexão, o mesmo problema que o `conn_task` do driver MySQL resolve (D37).

A UI `egui` não pode bloquear, e o runtime `tokio` não pode ter workers presos em IO de disco.

## Opções Consideradas
1. `rusqlite` (feature `bundled`) com uma thread de SO dedicada por sessão, comandos por `tokio::sync::mpsc` e respostas por `oneshot`.
2. `rusqlite` com `tokio::task::spawn_blocking` por comando.
3. `sqlx` com o backend `sqlite`.
4. Usar a lib SQLite do sistema (`libsqlite3-sys` sem `bundled`).

## Decisão Escolhida
Escolhemos a **Opção 1**. O novo crate `crustdb-sqlite` segue a direção `app -> drivers -> core` e implementa `Driver`/`Session`/`Introspector`/`CancelHandle` do `crustdb-core`. A API pública do core não muda de forma: só ganha as variantes `DriverKind::Sqlite` e `Product::Sqlite`.

* Cada sessão tem uma `std::thread` dona da `rusqlite::Connection`. Ela atende `ConnCmd` (mesmo enum e mesmo laço `next` do `crustdb-mysql/src/conn_task.rs`) com `blocking_recv`. Enquanto um `Rows` está ativo, a thread só atende `Fetch`/`Finish`. Qualquer outro comando encerra o resultado antes.
* Dependência: `rusqlite` com `--no-default-features --features bundled,column_decltype`. O `bundled` fixa a versão do SQLite e não exige `libsqlite3` no sistema. Sem as default features, o `sqlite-wasm-rs` não entra no grafo do `cargo deny`. Licenças: `rusqlite` e `libsqlite3-sys` são MIT, e o SQLite é domínio público.
* O caminho do arquivo vai em `ConnectTarget.database`. `host`, `port`, `user`, `tls` e `password` são ignorados. Sessões que não são do editor abrem com `SQLITE_OPEN_READ_ONLY`.
* `CancelHandle` guarda um `rusqlite::InterruptHandle` (`Send + Sync`), e `SQLITE_INTERRUPT` vira `ErrorKind::Cancelled`.
* `explain_rows` devolve `Ok(None)`. Pela regra existente (§4.7, estimativa desconhecida -> Password, sem senha -> D25), toda DML com estimativa fica no caminho mais restritivo. É fail-safe e não muda o core.

### Consequências Positivas
* O runtime `tokio` e a UI nunca bloqueiam em IO do SQLite.
* A estrutura espelha o driver MySQL, então revisão e manutenção usam um padrão conhecido.
* Não há dependência de biblioteca do sistema, e o empacotamento `.deb`/`.rpm` não muda.

### Consequências Negativas / Trade-offs
* Cada sessão aberta consome uma thread de SO. Isso é aceitável para um cliente desktop com poucas sessões por workspace.
* Compilar o SQLite em C (`cc`) aumenta o tempo do build limpo no CI.
* Toda DML no editor SQLite pede a confirmação mais forte até existir uma regra de estimativa própria.
* Adicionar a variante em `DriverKind` obriga a atualizar todos os `match` exaustivos (core, testkit e app), e um `connections.json` com `"sqlite"` não abre em versões antigas.
* Fora do escopo inicial: bancos `ATTACH`, índices/FKs na árvore, `row_estimate` via `sqlite_stat1`, criação de arquivo novo pela UI, criptografia (SQLCipher) e extensões carregáveis.
