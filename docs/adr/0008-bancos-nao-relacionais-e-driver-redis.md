# Bancos não relacionais e driver Redis

* **Status:** Aceito
* **Data:** 2026-10-30
* **Issue Relacionada:** #62

## Contexto e Problema
A issue #62 adiciona suporte ao Redis. Os bancos NoSQL planejados são #121 (MongoDB), #122 (DynamoDB) e #123 (Cassandra). A #9 é guarda-chuva de bancos **relacionais** e não entra aqui. Este ADR fixa o núcleo reaproveitável pelos NoSQL e o driver Redis. Os ADRs de #121/#122/#123 devem estender este.

O Redis difere dos drivers atuais assim:

* Não há SQL, schema, tabela, DDL, `EXPLAIN` nem transação SQL. Há chaves, tipos de valor (string, hash, list, set, zset, stream) e comandos.
* Chaves são binárias (não necessariamente UTF-8) e podem passar de 100 mil. Listar tudo de uma vez trava a UI e o servidor. O único percurso seguro é `SCAN` com cursor opaco, que pode repetir chaves.
* Os "namespaces" são os bancos numerados (`SELECT n`). Não existe `CREATE DATABASE`.
* Não há cancelamento no servidor. Um comando lento (Lua, `KEYS`) segue rodando até terminar.
* Segurança: `FLUSHALL`, `FLUSHDB` e `CONFIG SET` não têm equivalente no AST do `sqlparser`.

Fatos do código (verificados):

1. `Driver` (`crates/crustdb-core/src/driver.rs:92-93`) exige `dialect()` e `introspector()`. `Session` tem `start`, `start_params`, `explain_rows`, `use_database` e `CancelHandle`. `dialect()` é usado em `actors/data.rs:274` e `actors/export/mod.rs:235`. `introspector()` é usado em `actors/workspace.rs`, `actors/workspace/diagram.rs`, `actors/editor/mod.rs:342` e `actors/data.rs`.
2. `DriverKind { Postgres, Mysql, Sqlite }` está em `crates/crustdb-core/src/model.rs:8` e é serializado em `connections.json`. Há `match` exaustivos no app (`state/tree.rs:64`, `state/connection_form.rs:45`, `state/tree_home.rs:14`, `ui/home.rs:19` e `:27`, `ui/connection_form.rs:154`, `export_fmt.rs:42`), no core (`sql/mod.rs:68-74` com `dialect(kind)`, `sql/keywords.rs:164-169`, `query.rs:131-138` com `operators_for`) e no testkit (`crates/crustdb-testkit/src/fake_driver.rs:143-147`). Código de produção não usa `unreachable!` hoje (o único está em teste, `runtime/mod.rs:279`).
3. `DriverRegistry` é `Vec<Arc<dyn Driver>>` (`crates/crustdb-app/src/runtime/mod.rs:19`).
4. O túnel SSH fica no app: ele preenche `ConnectTarget.connect_addr`, e `host` continua sendo o nome TLS (`driver.rs:32-37`).
5. O teste `unknown_scheme_rejected` (`crates/crustdb-core/src/url.rs:219`) usa `redis://h`, e o plano o cita em `docs/specs/2026-10-17-crustdb-plan.md:247`. Ambos passam a usar `mongodb://h`.
6. `Verdict { Run, Confirm, Password }` (`safety.rs:37`) é reaproveitável. `Classified`/`StatementClass` não são, pois dependem do AST do `sqlparser`.
7. "Frame" é a meta de UI de até 16 ms (spec §7, `docs/specs/2026-10-17-crustdb-spec.md:326`; plano, linhas 699-702). É o critério de "100k chaves sem travar".
8. `deny.toml` permite Apache-2.0, MIT, BSD-2/3, ISC, MPL-2.0, Zlib, Unicode-3.0 e BUSL-1.1. As licenças de `redis` (BSD-3-Clause) e `fred` (MIT) estão aqui **de memória** e devem ser confirmadas com `cargo deny check` na fase de implementação.

## Opções Consideradas
1. O Redis implementa `Driver` e `Session` com stubs (`dialect()`/`introspector()` vazios ou com erro). **Rejeitada:** viola LSP (o contrato SQL é prometido e não cumprido) e ISP (o cliente é forçado a depender do que não usa). Cada chamada nova de `dialect()` viraria um risco de erro em runtime.
2. Traits separados `StoreDriver`/`StoreSession` no core, mais `DataModel` e `Capabilities` para a UI decidir por capacidade. **Escolhida.**
3. Um trait por banco, sem núcleo comum. **Rejeitada:** cada NoSQL (#121/#122/#123) reimplementaria árvore, leitura limitada, paginação e classificação de risco. Zero reuso.
4. Cliente: `redis` (redis-rs) ou `fred`. Ver D5.

## Decisão Escolhida
Escolhemos a **Opção 2**. O `Driver` SQL fica inalterado (OCP). O core ganha um módulo de armazenamento chave-valor, e a UI passa a depender de `Capabilities` em vez de `match` espalhado por `DriverKind` (DIP).

### D1. Um só `DriverKind`, com modelo de dados
Adicionar `DriverKind::Redis` (serde `"redis"`) e `Product::Redis`. No core, `pub enum DataModel { Relational, KeyValue }` e `DriverKind::data_model()` (na #164) com `match` exaustivo, sem braço `_`. Variantes `Document` e `WideColumn` só entram nos ADRs de #121/#122/#123 (YAGNI).

### D2. A UI decide por capacidade
`pub struct Capabilities` só com `bool`s, obtida por `const fn DataModel::capabilities()`:

* `table_tab` (Data/Structure/DDL), `er_diagram`, `table_export_csv`;
* `sql_editor` (beautify, completion, explain, transação);
* `row_edit` (ADRs 0003/0004), `ai_query` (ADR 0005);
* `command_editor`.

Para `KeyValue` tudo é `false`, exceto `command_editor`. A UI desabilita a ação com tooltip "Not supported for Redis". Nunca usa `unreachable!`/`panic!`. Os `match` existentes ganham um braço `Redis` explícito. Nos helpers SQL do core (`sql::dialect`, `keywords`, `operators_for`) o braço Redis devolve um valor inerte (`GenericDialect`, lista de palavras-chave vazia, `COMPARE`). Ele é **inalcançável** em produção, pois `Capabilities` desliga `sql_editor`, `table_tab` e `row_edit`, de modo que nenhum caminho SQL é aberto para Redis. A diferença para o stub da Opção 1: aqui o valor inerte é só o braço total de um `match` sobre enum fechado, sem implementar `Driver` nem prometer contrato SQL. O `fake_driver` do testkit é só de `Driver` SQL e recebe um braço equivalente (`GenericDialect`). Um teste (`redis_kind_never_reaches_sql_helpers`) garante que `capabilities()` de `KeyValue` desliga todas as telas que os usam.

### D3. Traits `store` (ISP + DIP)
Novo módulo `crustdb-core::store`, sem IO, com `async_trait` como o resto do core:

* `StoreDriver { kind(); connect(&ConnectTarget, SessionPurpose) -> Box<dyn StoreSession>; assess(&[Vec<u8>]) -> Result<(Verdict, String), Refused> }`. `Refused` é um erro tipado (`thiserror`) com o motivo (ver D9)
* `StoreSession { server_version(); ping(); namespaces(); scan(&ScanRequest) -> ScanPage; read(&EntryKey, limit) -> EntryView; run(&[Vec<u8>]) -> Reply; close() }`

Tipos neutros, reaproveitáveis por outros NoSQL:

* `EntryKey(Vec<u8>)`, binary-safe. A exibição usa `String::from_utf8_lossy`.
* `ScanRequest { cursor: Option<String> opaco, pattern, count }` e `ScanPage { entries: Vec<KeyEntry>, next }`, com `KeyEntry { key, kind, ttl }`.
* `ValueKind { Text, Map, List, Set, SortedSet, Stream, Other(String) }`.
* `EntryView { ttl: Option<Duration>, size_bytes: Option<u64>, body: ValueBody, truncated: bool }`.
* `Reply`: árvore estilo RESP3.

Não há `CancelHandle` (ver D8). No app, `DriverRegistry::get(kind)` devolve `enum Backend { Sql(Arc<dyn Driver>), Store(Arc<dyn StoreDriver>) }`. Os atores SQL seguem com `Arc<dyn Driver>`, e um ator novo atende `Store`. Dependências: `app -> crustdb-redis -> core`, e o core continua sem `tokio` e sem `egui`.

### D4. Funções puras no core
* `store::keyspace::group(keys, sep) -> árvore por prefixo`. O separador é configurável por conexão (padrão `":"`).
* `ValueBody::to_rows() -> (Vec<ColumnMeta>, Vec<Row>)`, para reaproveitar o grid de resultado: hash -> `field`/`value`, zset -> `member`/`score`, list -> `index`/`value`, stream -> `id`/`fields`.
* Strings respeitam `CELL_MEMORY_CAP` (`model.rs:3`).

### D5. Cliente `redis` (redis-rs)
* `cargo add redis --no-default-features --features tokio-comp`, sem features de TLS nem de cluster.
* Uma `MultiplexedConnection` por `StoreSession`, com os timeouts de `ConnectTarget.connect_timeout`.
* Comandos sempre como `Cmd::new()` com `.arg(bytes)` em cada argumento, inclusive o nome do comando (`redis::cmd(&str)` não é binary-safe), retornando `redis::Value`, sem helpers tipados (uniforme com `run()`).
* TLS com SSH: o driver abre o próprio TCP para `connect_addr.unwrap_or(host:port)` e o envolve com `tokio-rustls`, com `ServerName` = `host` (builder como `crates/crustdb-postgres/src/connect.rs`). O stream segue para `MultiplexedConnection::new_with_config`. API confirmada na fase de código (redis 1.8.1, ver "Ajustes (#165)"); o plano B (`tokio-rustls-comp` e recusar "SSH + VerifyFull") foi descartado.
* Por que não `fred` (MIT): pool, reconexão e retry automáticos conflitam com "cancelar = descartar a conexão" (D8). Traz mais dependências e foca em cluster/sentinel (fora de escopo). A vantagem dele, registrada aqui, é o `tls_server_name` nativo.

### D6. Conexão
* `parse_connection_url` aceita `redis://` e `rediss://`. `rediss` usa `VerifyFull` por padrão. `VerifyCa` é tratado como `VerifyFull` (fail-safe). `Require` aceita o certificado sem verificação.
* `user` é o usuário ACL. URL só com senha (`redis://:p@h`) usa `default`.
* O path `/n` vai para `ConnectTarget.database` e deve ser `u32`, senão `Malformed`. A conexão faz `AUTH` (via redis-rs) e `SELECT n`. Porta padrão 6379.
* `namespaces()` usa `CONFIG GET databases`. Qualquer erro do servidor nesse comando (ACL, `CONFIG` renomeado/desabilitado) cai em `INFO keyspace` somando o banco atual; erro de I/O nunca dispara o fallback.
* Um `connections.json` com `"redis"` não abre em versões antigas (mesmo trade-off do ADR 0002).

### D7. Árvore e leitura sem travar
* Só `SCAN cursor MATCH p COUNT n`, com cursor opaco e lotes de 500 (`ROWS_BATCH`). **Nunca `KEYS`.** `SCAN` pode duplicar, então o app deduplica.
* `TYPE` e `PTTL` por lote, via pipeline. `MEMORY USAGE` só ao abrir a chave (negado -> `None`).
* Leitura sempre limitada: `GETRANGE 0 CELL_MEMORY_CAP`; `HSCAN`/`SSCAN` ou `ZRANGE 0 limit WITHSCORES`; `LRANGE 0 limit`; `XRANGE - + COUNT limit+1`. Tipo desconhecido (módulos) não é lido: `scan` o lista como `ValueKind::Other` e `read` devolve `Unsupported`; **não há `JSON.GET`** (prevalece #163.7). Regra única: pedir sempre `limit + 1` itens (sentinela); se vierem mais de `limit`, descartar o excedente e marcar `truncated = true`. Para `GETRANGE`, pedir `CELL_MEMORY_CAP + 1` bytes. `HSCAN`/`SSCAN` acumulam até `limit + 1`.

### D8. Cancelamento
Não há cancel no servidor. Durante o `.await` de `run()`/`scan()` o ator tem `&mut` da sessão, então ele não consegue descartá-la de fora. Mecanismo: a sessão vive numa task dona, e a task dona faz `tokio::select!` entre a operação e o canal de cancelamento, e o ator envia o cancelamento (alternativa equivalente: `abort()` da task dona). No cancelamento a task dona sai do `select!` e faz o drop do `Box<dyn StoreSession>`; o ator reconecta sob demanda. No driver, descartar a `MultiplexedConnection` com comando em voo **não** fecha o socket (o `poll_close` do pipeline espera as respostas pendentes). Por isso `RedisSession` guarda o `JoinHandle` do laço de I/O e o `Drop` faz `abort()`, o que derruba o socket na hora. Pipelines usam `&mut conn`, sem clones. O teste `dropping_session_closes_server_client` cobre sessão ociosa e sessão com `WAIT 1 5000` descartada em voo (o cliente precisa sumir do `CLIENT LIST` antes de 5 s). Trade-off: Lua ou comando lento segue no servidor. `SCRIPT KILL` fica fora de escopo.

### D9. Somente leitura e comandos perigosos
* Sessões Metadata, Data e Export recusam `run()`. O driver garante isso.
* `crustdb_redis::classify(argv)` é exposta por `StoreDriver::assess` como `Result<(Verdict, String), Refused>`. Não existe `CommandClass`: `Verdict` (Run/Confirm/Password) mais o motivo cobrem a classificação, e `Err(Refused)` cobre "Recusado". Reaproveita o `Verdict` e o fluxo do §4.7. `run()` também devolve o mesmo erro tipado se receber um comando recusado, sem depender de a UI ter chamado `assess`.
  * **Password:** `FLUSHALL`, `FLUSHDB`, `SHUTDOWN`, `MODULE LOAD`/`MODULE LOADEX` (carregam código nativo no servidor, pior que script) e `EVAL`/`EVALSHA`/`EVAL_RO`/`EVALSHA_RO`/`FCALL`/`FCALL_RO`. O script não é analisável (`EVAL "return redis.call('FLUSHALL')" 0` contornaria o Password), então todo script é tratado como o pior caso. Aceitamos o custo: a variante `_RO` também pede senha, por não termos como provar que o script é de leitura.
  * **Confirm:** `CONFIG SET|REWRITE|RESETSTAT`, `DEBUG`, `KEYS`, `SCRIPT FLUSH`, `FUNCTION FLUSH|DELETE`, `CLIENT KILL`, `MIGRATE`, `REPLICAOF`/`SLAVEOF`, `ACL SETUSER|DELUSER`, demais subcomandos de `MODULE`, `SAVE`, `SWAPDB`, `SCRIPT LOAD`, `FUNCTION LOAD`; `DEL`/`UNLINK` com mais de 10 chaves (`DEL_CONFIRM_ABOVE`); `XREAD`/`XREADGROUP` sem `BLOCK`; comando desconhecido.
  * **Recusados (fora de escopo):** `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`, `SUBSCRIBE`/`PSUBSCRIBE`/`MONITOR`, `B*POP`, `XREAD BLOCK`, `WAIT`/`WAITAOF` (bloqueiam como `B*POP`) e `SYNC`/`PSYNC`/`REPLCONF` (transformam o socket em stream de replicação). Também recusados por quebrarem o pareamento ou o estado da conexão multiplexada: `CLIENT REPLY`, `HELLO`, `RESET`, `QUIT`. `SELECT` e `AUTH` via `run()` são recusados: o namespace e a identidade da sessão só mudam por `connect` ou pelo seletor de namespace (que reconecta com `SELECT n`), nunca por comando livre. Escolha registrada para evitar que um `SELECT` no editor mude o banco de forma silenciosa para as leituras seguintes.
* Tokenizador estilo `redis-cli` (aspas, `\xHH`). Cada argumento vai como bulk RESP. **Nunca concatenar** (anti-injeção). Sem `quote_ident` nem `Dialect`.

### D10. Fora de escopo
Cluster, Sentinel, Pub/Sub, `MULTI`/`EXEC`, edição de valores, mTLS e `SCRIPT KILL`.

### Consequências Positivas
* O `Driver` SQL e os atores SQL não mudam. Nenhum stub viola LSP/ISP.
* MongoDB, DynamoDB e Cassandra reaproveitam `store`, `Capabilities`, `keyspace::group`, `to_rows` e o fluxo de `Verdict`. Cada um só acrescenta uma variante de `DataModel`/`ValueKind` e seu crate.
* A UI não trava em chaves grandes: lotes de 500, leitura limitada e `KEYS` nunca usado.
* O gate por `Capabilities` troca `match` espalhados por uma única tabela, com `const fn` testável.
* Lógica pura (agrupamento, linhas, classificação, tokenizador) é testável sem Docker.

### Consequências Negativas / Trade-offs
* Dois caminhos no app (`Backend::Sql` e `Backend::Store`) e um ator a mais para manter.
* Adicionar `DriverKind::Redis` obriga a atualizar os `match` exaustivos listados no fato 2, e o `connections.json` fica incompatível com versões antigas.
* Cancelar descarta a conexão, mas não interrompe o comando no servidor.
* TLS com SSH exige código próprio (`tokio-rustls`), duplicado do driver Postgres em `crustdb-redis/src/tls.rs`.
* `Capabilities` pode crescer com cada NoSQL. Novas flags só entram com consumidor real.
* Licenças confirmadas com `cargo deny check`: `redis` entra sem exceção; só o transitivo `xxhash-rust` (BSL-1.0) precisou de exceção em `deny.toml`.

## Plano de Testes
Só comportamento do domínio. Proibido testar apenas redis-rs ou serde sem regra do CrustDB. Testes seguem AAA.

**Core (unit):** `parses_rediss_with_acl_user_and_db` (`rediss://alice:p%40ss@h:6380/3` -> Redis, `alice`, `p@ss`, db `"3"`, `VerifyFull`); `redis_password_only_url_uses_default_user`; `redis_db_path_must_be_number`; `redis_verify_ca_is_treated_as_verify_full`; `redis_require_skips_verification`; `redis_kind_never_reaches_sql_helpers`; `unknown_scheme_rejected` (agora com `mongodb://h`); `key_value_model_disables_sql_screens`; `groups_keys_by_separator_prefix`; `custom_separator_groups_by_it`; `key_without_separator_is_leaf`; `non_utf8_key_keeps_raw_bytes`; `hash_body_maps_to_field_value_rows`; `zset_rows_keep_score_order`.

**`crustdb-redis` (unit, sem Docker):** `flushall_requires_password`; `config_get_runs_but_config_set_confirms`; `del_over_key_limit_confirms`; `unknown_command_confirms`; `command_name_is_case_insensitive`; `subscribe_monitor_multi_rejected`; `eval_cannot_bypass_flushall_password` (`EVAL`, `EVALSHA`, `FCALL` e variantes `_RO` dão Password); `script_load_and_function_load_confirm`; `connection_state_commands_rejected` (`CLIENT REPLY`, `HELLO`, `RESET`, `QUIT`, `SELECT`, `AUTH`; e `run()` também recusa); `tokenizer_keeps_quoted_crlf_as_single_arg` (`SET k "v\r\nFLUSHALL"` resulta em exatamente 3 argumentos); `unterminated_quote_errors`.

**`crustdb-redis` (integração, `#[ignore = "docker"]`, testcontainers `GenericImage` com `redis:latest` ou `CRUSTDB_REDIS_IMAGE`):** `scan_visits_all_10k_keys_in_bounded_pages`; `reads_each_type_with_ttl_and_size`; `large_hash_read_is_truncated_at_limit`; `binary_key_round_trips`; `select_db_isolates_keyspace`; `acl_user_without_config_still_lists_namespaces`; `wrong_password_maps_to_auth_error`; `non_editor_session_rejects_run`; `dropping_session_closes_server_client` (`CLIENT LIST` numa segunda sessão); TLS com CA opcional.

**App (`StoreDriver` falso no `crustdb-testkit`):** `redis_connection_disables_table_diagram_export_actions`; `keyspace_loads_lazily_in_scan_batches`; `duplicate_scan_entries_deduped`; `flushdb_in_editor_opens_password_dialog`; perf: frame de até 16 ms com 100k chaves (`tests/perf.rs`).

## Issues Filhas Sugeridas
Apenas listadas, não criadas aqui.

1. `feat(core)`: `DriverKind::Redis`, `DataModel`, `Capabilities`, URL, traits `store`, `keyspace::group`, `to_rows`.
2. `feat(drivers)`: crate `crustdb-redis` (TLS/SSH/ACL/`SELECT`, `namespaces`, `scan`, leitura limitada, `run`, `classify`, tokenizador, integração).
3. `feat(app)`: `Backend` no `DriverRegistry`, ator Store, formulário (Redis, 6379, separador), gate por `Capabilities`.
4. `feat(ui)`: árvore de keyspace lazy e visualizador somente leitura.
5. `feat(ui)`: editor de comandos e diálogo de confirmação/senha.
6. Depois: edição de valores, Cluster/Sentinel, e ADRs de #121/#122/#123 reaproveitando `store`.

## Ajustes da fase de código (#163)

1. `Capabilities` é por `DataModel` (`DataModel::capabilities()`), não por `DriverKind`. A #164 acrescenta `DriverKind::data_model()`. O teste é `key_value_model_disables_sql_screens` (mais `relational_model_keeps_sql_screens`).
2. `scan` e `read` não recebem `ns`: a sessão fica presa a `ConnectTarget.database`, coerente com D9 (namespace só muda por connect).
3. `server_version()` segue devolvendo `&ServerVersion`; `Product::Redis` vem na #164.
4. `impl From<Refused> for DriverError` (`ErrorKind::Unsupported`) fica no core, pela regra de órfãos.
5. TTL é `Option<Duration>` (`None` = sem expiração), sem tipo `Ttl`.
6. `ScanPage.entries` é `Vec<KeyEntry { key, kind, ttl }>`; `EntryView` não tem `kind`.
7. Sem `ValueKind::Document` nem `ValueBody::Document/Other` (YAGNI): `ValueBody` tem `Text, Map, List, Set, SortedSet, Stream`. Tipo desconhecido na leitura vira `DriverError` `Unsupported` no driver.

## Ajustes da fase de código (#164)

* D-a. O seletor de driver do formulário não ganha `selectable_value` para Redis; só `selected_text` ganha o braço "Redis". A opção entra com o driver (fase 2b).
* D-b. `ConnectionForm::import_url` recusa URL de modelo não relacional (`UnsupportedScheme`) até o backend `Store` existir. Esse gate sai na #166.
* D-c. `redis://` puro usa `Disable` e ignora `sslmode`. `rediss://` usa `Require` com `sslmode=require` (e `prefer`/`allow`, com aviso) e `VerifyFull` em qualquer outro caso (ausente, `verify-ca`, `verify-full`, `disable`, valor desconhecido).
* D-d. `VerifyCa` vira `VerifyFull` só no parser. O driver da fase 2b também deve tratar `VerifyCa` escolhido à mão no formulário (como o MySQL em `tls_notes`).
* D-e. Braços inertes nos helpers SQL (`sql::dialect`, `keywords`, `operators_for`, `export_fmt::dialect`, árvore) e no `FakeDriver`: hoje são inalcançáveis só porque nenhum driver Redis está registrado; nada na UI usa `capabilities()` ainda. O gate de `Capabilities` (#166) precisa entrar antes ou junto com o registro do driver Redis no `DriverRegistry` (#165/#166).

## Ajustes da fase de código (#165)

* E-a. `redis` 1.8.1, sem features de TLS. O plano A de TLS foi confirmado: `MultiplexedConnection::new_with_config(&RedisConnectionInfo, stream, AsyncConnectionConfig)` aceita qualquer `AsyncRead + AsyncWrite + Unpin + Send`. O driver abre o `TcpStream` para `connect_addr.unwrap_or(host:port)`, embrulha com `tokio_rustls::TlsConnector` usando `ServerName` = `host`, e faz `tokio::spawn` do future de I/O. "SSH + VerifyFull" funciona; plano B descartado.
* E-b. `Drop` de `RedisSession` faz `abort()` do laço de I/O: descartar a conexão com comando em voo não fecha o socket (ver D8). Pipelines usam `&mut conn`.
* E-c. `AsyncConnectionConfig::set_response_timeout(None)`: o padrão de 500 ms quebraria comandos lentos, e cancelar é descartar a sessão. TCP, TLS, `AUTH`/`SELECT` e `INFO server` rodam sob um único `tokio::time::timeout(connect_timeout)`.
* E-d. Sem `JSON.GET`: prevalece #163.7. Tipo desconhecido vira `ValueKind::Other` no `scan` e `Unsupported` no `read`. `MEMORY USAGE` sem `SAMPLES`; erro do servidor vira `size_bytes: None`, erro de I/O propaga.
* E-e. `DEL_CONFIRM_ABOVE = 10`: `DEL`/`UNLINK` com mais de 10 chaves pedem confirmação.
* E-f. Fallback de `namespaces()` para `INFO keyspace` em qualquer erro de servidor de `CONFIG GET databases` (D6).
* E-g. `ConnectTarget.database` que não seja `u32` falha em `connect` com `ErrorKind::Connect` ("database must be a numeric index"); vazio equivale a `0`. Usuário vazio vira `default` quando há senha.
* E-h. `tls.rs` é cópia enxuta do TLS do Postgres (YAGNI: dois consumidores). `VerifyCa` vira `VerifyFull`; `Require` usa um verificador que aceita qualquer certificado mas valida as assinaturas do handshake; TLS antigo devolve a mensagem D39. **Issue de extração a abrir** (crate/módulo compartilhado) se surgir um terceiro consumidor.
* E-i. Erros: `AuthenticationFailed` ou `NOAUTH`/`WRONGPASS` viram `ErrorKind::Auth`; erro do servidor vira `Query { code }`; timeout de I/O vira `Timeout`; falha de I/O vira `Connect`; handshake TLS vira `Tls`; o resto vira `Protocol`.
* E-j. Licença: `xxhash-rust` (BSL-1.0, dependência transitiva do `redis`) tem exceção em `deny.toml`. `XREAD`/`XREADGROUP` sem `BLOCK` são `Confirm`; com `BLOCK`, recusados. `DISCARD` e `UNWATCH` são recusados junto com `MULTI`/`EXEC`/`WATCH`.
* E-k (revisão). `WAIT`/`WAITAOF`/`SYNC`/`PSYNC`/`REPLCONF` recusados; `MODULE LOAD`/`LOADEX` exigem senha. `TYPE`/`PTTL` do `scan` rodam com `ignore_errors`: chave negada pela ACL (o `SCAN` não checa padrão de chave) é descartada, não derruba a página. O teste de cancelamento com comando em voo usa `WAIT` via `RedisSession::call` interno (teste de unidade `#[ignore = "docker"]`), porque `run` agora recusa `WAIT` e `DEBUG SLEEP` travaria o servidor inteiro (o observador também).
* Fora desta fase: o driver não está registrado no `DriverRegistry`/`main.rs`, e os comentários "unreachable: no Redis driver" do core e do app seguem como estão.

**NÃO verificado em Docker nesta fase:** os testes de integração (`crates/crustdb-redis/tests/integration.rs`, `#[ignore = "docker"]`) e o helper `start_redis` foram escritos e compilam, mas não foram executados (sem Docker no ambiente). Rodar `cargo test -p crustdb-redis -- --include-ignored` antes de confiar em TLS, cancelamento e `SCAN`.
