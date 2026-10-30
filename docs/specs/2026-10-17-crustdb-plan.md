# cRustDB v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Construir o cRustDB v1: client desktop Linux (egui) para PostgreSQL e MySQL/MariaDB, com grid somente leitura, editor SQL raw e política de segurança contra escrita acidental.

**Architecture:** Workspace Cargo com `crustdb-core` (domínio puro, sem IO), `crustdb-postgres` e `crustdb-mysql` (implementam os traits do core) e `crustdb-app` (UI egui + runtime tokio com atores de sessão + storage). A UI nunca bloqueia: envia `Command` e recebe `Event` por canais. Tudo que o banco devolve trafega como texto, mais metadado de tipo.

**Tech Stack:** Rust stable mais recente (edition 2024), eframe/egui (glow), egui_extras, egui-phosphor, tokio, tokio-postgres, tokio-postgres-rustls, mysql_async (rustls), rustls, rustls-native-certs, sqlparser, russh, keyring, secrecy, serde/serde_json, url, uuid, thiserror, tracing, rfd, async-trait. Dev: testcontainers, rcgen, criterion, egui_kittest, cargo-llvm-cov, cargo-deny.

**Spec:** `docs/specs/2026-10-17-crustdb-spec.md`. Leia a spec inteira antes da Task 1. Este plano cita a spec como `§n` (seção) e `Dn` (decisão).

## Global Constraints

- Plataforma: Linux x86_64, X11 e Wayland. Tema escuro fixo. UI em inglês.
- Bancos: PostgreSQL ≥ 12; MySQL ≥ 5.6.5; MariaDB ≥ 10.0.5. Se o `mysql_async` falhar no piso, o piso sobe para a menor versão que passar na matriz (D6), registrada no README.
- TLS: só `rustls` (TLS ≥ 1.2). Modos `disable`/`require`/`verify-ca`/`verify-full`, padrão `verify-full`, sem `prefer` (D10, D39).
- `unsafe_code = "forbid"` em todo o workspace. Nada de `unwrap`/`expect` fora de testes (`clippy::unwrap_used`, `clippy::expect_used` = deny).
- Erros tipados com `thiserror` no core e nos drivers; `anyhow` só em `crates/crustdb-app/src/main.rs`.
- Segredos só em `secrecy::SecretString` e no keyring; nunca em disco, log ou histórico. `Debug` redigido.
- `crustdb-core` não depende de tokio, egui nem de qualquer IO de rede/arquivo.
- Valores do usuário em SQL gerado: sempre parâmetros bound. Identificadores: sempre `Dialect::quote_ident`.
- Constantes: `CELL_MEMORY_CAP = 1_048_576` bytes; `GRID_DISPLAY_CHARS = 256`; página padrão 300 (10–10.000); `editor_row_limit` padrão 1000 (10–100.000); `safety_threshold` padrão 1000 (≥ 1); lote de `Event::Rows` ≤ 500 linhas; ping a cada 60 s; timeout de conexão 10 s; cursor pausado ocioso fecha em 5 min; histórico ≤ 10.000 entradas.
- Arquivos: `$XDG_CONFIG_HOME/crustdb/connections.json` (0600), `$XDG_CONFIG_HOME/crustdb/known_hosts`, `$XDG_DATA_HOME/crustdb/history/<connection-id>.jsonl` (0600), `$XDG_STATE_HOME/crustdb/crustdb.log`; diretórios 0700.
- Licença `Apache-2.0` em todo `Cargo.toml`; `deny.toml` só aceita licenças compatíveis.
- Git: repositório `https://github.com/Desbravando-Rust/crustdb`, uma branch por milestone (`feat/m0-foundation`, `feat/m1-core`, …) a partir de `main`, um PR por milestone. Nunca push direto na `main`.
- **Commits e push só com aprovação explícita do usuário** (CLAUDE.md global). Antes de pedir: mostrar resumo do diff, testes executados e pendências. Mensagens no estilo Conventional Commits, **sem mencionar IA/Claude, sem `Co-Authored-By`**.
- Arquivos de código ≤ 500 linhas; dividir antes de passar disso.
- Desvio consciente da spec: um 5º crate só de testes, `crates/crustdb-testkit` (`publish = false`, dev-dependency), para não duplicar certificados e containers entre os dois drivers. O binário continua com os 4 crates de D36.

## Review Focus

1. **Nomes de identificadores incomuns** (aspas, espaço, unicode, palavra reservada, maiúsculas no PG): abrir a tabela e filtrar devem funcionar. Teste `table_with_quoted_mixed_case_name_opens` nas Tasks 13 e 16; golden `quote_ident_doubles_quote_char` na Task 7.
2. **Bytes que não são UTF-8 em coluna de texto** (MySQL latin1, dados corrompidos): exibir com `U+FFFD`, sem crash. `invalid_utf8_text_displays_lossy` na Task 2; `latin1_column_displays` na Task 15.
3. **Queda do servidor no meio de um cursor pausado ou de uma página**: erro visível, sessão reconectável, sem travar a UI. `server_restart_during_paused_cursor_reports_error` nas Tasks 12 e 15.
4. **Cursor e seleção com caracteres multibyte** (acentos, emoji) no editor: o statement certo roda. `statement_at_with_multibyte_text` na Task 3; `char_to_byte_offset_roundtrip` na Task 30.
5. **Tabela larga (250 colunas) ou célula enorme (50 MB)**: grid responsivo e memória limitada. `grid_caps_cell_transfer_at_1mib` na Task 12; `wide_table_frame_time` na Task 36.

---

## Milestone M0: Foundation (branch `feat/m0-foundation`)

### Task 1: Workspace, toolchain e quality gates

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.github/workflows/ci.yml`, `README.md`
- Create: `crates/crustdb-core/Cargo.toml`, `crates/crustdb-core/src/lib.rs`
- Create: `crates/crustdb-postgres/Cargo.toml`, `crates/crustdb-postgres/src/lib.rs`
- Create: `crates/crustdb-mysql/Cargo.toml`, `crates/crustdb-mysql/src/lib.rs`
- Create: `crates/crustdb-testkit/Cargo.toml`, `crates/crustdb-testkit/src/lib.rs`
- Create: `crates/crustdb-app/Cargo.toml`, `crates/crustdb-app/src/main.rs`

**Interfaces:**
- Produces: workspace compilável; binário `crustdb` que abre uma janela vazia com o título `cRustDB`.

- [ ] **Step 1:** `git checkout -b feat/m0-foundation`. Rodar `rustup update stable && rustc --version` e fixar essa versão exata em `rust-toolchain.toml` (`components = ["rustfmt", "clippy", "llvm-tools-preview"]`).
- [ ] **Step 2:** Criar o `Cargo.toml` raiz: `members = ["crates/*"]`, `resolver = "3"`, `[workspace.package]` (`edition = "2024"`, `license = "Apache-2.0"`, `rust-version` = versão fixada), `[workspace.lints.rust] unsafe_code = "forbid"`, `[workspace.lints.clippy] unwrap_used = "deny"`, `expect_used = "deny"`, `[profile.release] lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`. Cada crate usa `lints.workspace = true`. Adicionar dependências com `cargo add` (versão mais recente compatível; não digitar versões de memória).
- [ ] **Step 3:** No `main.rs`, `eframe::run_native("cRustDB", …)` com `ViewportBuilder` de título `cRustDB` e `egui::Visuals::dark()`. Testes de módulo usam `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- [ ] **Step 4:** `deny.toml`: advisories deny, licenças permitidas `Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-3.0`, `Zlib`, `MPL-2.0` (só via dependência). CI (`ci.yml`): jobs `fmt`, `clippy`, `test`, `deny`, `coverage` (stub até a Task 35) e `integration` (matriz de imagens; stub até a Task 10).
- [ ] **Step 5:** Verificar: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check && cargo build --release`. Esperado: tudo verde. `cargo run -p crustdb-app` abre a janela escura.
- [ ] **Step 6:** Pedir aprovação e commitar `chore: scaffold workspace, toolchain and CI gates`.

---

## Milestone M1: Core (branch `feat/m1-core`)

### Task 2: Modelo de domínio, erros, traits de driver e formatação de células

**Files:**
- Create: `crates/crustdb-core/src/model.rs`, `error.rs`, `driver.rs`, `format.rs`; Modify: `lib.rs`

**Interfaces:**
- Produces (`crustdb_core::model`):
  - `enum DriverKind { Postgres, Mysql }` (serde `"postgres"`/`"mysql"`); `enum Product { Postgres, Mysql, Mariadb }`
  - `struct ConnectionId(pub uuid::Uuid)`
  - `enum ObjectKind { Database, Schema, Table, View, MaterializedView, Function, Procedure, Trigger, Sequence, Type }`
  - `struct ObjectRef { database: String, schema: Option<String>, name: String, kind: ObjectKind }` (MySQL: `schema = None`)
  - `enum TypeCategory { Integer, Decimal, Float, Text, Bool, Date, Time, Timestamp, Json, Binary, Uuid, Other }`
  - `struct ColumnMeta { name: String, type_name: String, category: TypeCategory, nullable: Option<bool>, is_primary_key: bool }`
  - `struct ColumnDetail { meta: ColumnMeta, default: Option<String>, comment: Option<String> }`
  - `struct IndexInfo { name: String, columns: Vec<String>, unique: bool, primary: bool, method: Option<String> }`
  - `struct ForeignKeyInfo { name: String, columns: Vec<String>, ref_table: ObjectRef, ref_columns: Vec<String>, on_update: String, on_delete: String }`
  - `enum Cell { Null, Text { value: Box<str>, truncated: bool }, Bytes { value: Box<[u8]>, total_len: u64, truncated: bool } }` com `Cell::text_capped(&str) -> Cell`, `Cell::text_lossy_capped(&[u8]) -> Cell`, `Cell::bytes_capped(&[u8]) -> Cell`
  - `type Row = Box<[Cell]>`; `struct ServerVersion { product: Product, major: u32, minor: u32, patch: u32, raw: String }`; `struct Completion { affected_rows: Option<u64> }`
  - `const CELL_MEMORY_CAP: usize = 1_048_576; const GRID_DISPLAY_CHARS: usize = 256;`
- Produces (`error`): `struct DriverError { kind: ErrorKind, message: String }`; `enum ErrorKind { Connect, Auth, Tls, Timeout, Query { code: Option<String>, position: Option<usize> }, Cancelled, Unsupported, Protocol }`
- Produces (`driver`), com `#[async_trait]`:
  - `enum TlsMode { Disable, Require, VerifyCa, VerifyFull }` (serde kebab-case); `struct TlsConfig { mode: TlsMode, ca_path: Option<PathBuf> }`
  - `struct ConnectTarget { host: String, port: u16, connect_addr: Option<SocketAddr>, user: String, password: Option<SecretString>, database: Option<String>, tls: TlsConfig, connect_timeout: Duration }`. `host` é sempre o host real (nome TLS); `connect_addr` é o endereço do túnel, quando houver.
  - `enum SessionPurpose { Metadata, Data, Export, Editor }`. Todos menos `Editor` são read-only no banco.
  - `enum ExecMode { Read, Other }`; `struct Param(pub Option<String>)`; `struct ResultHeader { columns: Vec<ColumnMeta> }`; `struct Fetched { rows: Vec<Row>, exhausted: bool }`
  - `trait Driver: Send + Sync { fn kind(&self) -> DriverKind; fn dialect(&self) -> &dyn Dialect; fn introspector(&self) -> &dyn Introspector; async fn connect(&self, t: &ConnectTarget, p: SessionPurpose) -> Result<Box<dyn Session>, DriverError>; }`
  - `trait Session: Send { fn server_version(&self) -> &ServerVersion; fn cancel_handle(&self) -> Box<dyn CancelHandle>; fn has_active_result(&self) -> bool; async fn ping(&mut self) -> Result<Duration, DriverError>; async fn start(&mut self, sql: &str, mode: ExecMode) -> Result<ResultHeader, DriverError>; async fn start_params(&mut self, sql: &str, params: &[Param]) -> Result<ResultHeader, DriverError>; async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError>; async fn finish(&mut self) -> Result<Completion, DriverError>; async fn explain_rows(&mut self, sql: &str) -> Result<Option<u64>, DriverError>; async fn use_database(&mut self, name: &str) -> Result<(), DriverError>; async fn close(self: Box<Self>); }`
  - `trait CancelHandle: Send + Sync { async fn cancel(&self) -> Result<(), DriverError>; }`
  - `trait Introspector: Send + Sync`, cada método recebendo `s: &mut dyn Session`: `databases`, `schemas(database)`, `objects(database, schema: Option<&str>, kind)`, `columns(&ObjectRef) -> Vec<ColumnDetail>`, `indexes`, `foreign_keys`, `ddl(&ObjectRef) -> String`, `row_estimate(&ObjectRef) -> Option<u64>`
- Produces (`format`): `display_cell(&Cell, &ColumnMeta, max_chars: usize) -> Cow<'_, str>`, `pretty_json(&str) -> Option<String>`, `hex_preview(&[u8], n: usize) -> String`, `hex_dump(&[u8], max: usize) -> String`, `human_size(u64) -> String`

- [ ] **Step 1: Testes que falham** em `format.rs`/`model.rs`:
```rust
#[test] fn null_displays_as_null() { assert_eq!(display_cell(&Cell::Null, &col(TypeCategory::Text), 256), "NULL"); }
#[test] fn text_truncated_at_max_chars() { let c = Cell::text_capped(&"é".repeat(300)); assert_eq!(display_cell(&c, &col(TypeCategory::Text), 256).chars().count(), 257); /* 256 + '…' */ }
#[test] fn binary_shows_size_and_preview() { let c = Cell::bytes_capped(&[0xde, 0xad, 0xbe, 0xef]); assert_eq!(display_cell(&c, &col(TypeCategory::Binary), 256), "(binary 4 B) deadbeef"); }
#[test] fn cell_memory_capped_at_1mib() { let c = Cell::text_capped(&"a".repeat(CELL_MEMORY_CAP + 10)); assert!(matches!(c, Cell::Text { truncated: true, ref value } if value.len() == CELL_MEMORY_CAP)); }
#[test] fn invalid_utf8_text_displays_lossy() { let c = Cell::text_lossy_capped(&[b'a', 0xff, b'b']); assert_eq!(display_cell(&c, &col(TypeCategory::Text), 256), "a\u{FFFD}b"); }
#[test] fn human_size_formats() { assert_eq!(human_size(2_355), "2.3 KB"); assert_eq!(human_size(4), "4 B"); }
#[test] fn pretty_json_formats_objects() { assert_eq!(pretty_json(r#"{"k":1}"#).as_deref(), Some("{\n  \"k\": 1\n}")); }
```
- [ ] **Step 2:** `cargo test -p crustdb-core`. Esperado: FAIL (tipos inexistentes).
- [ ] **Step 3:** Implementar os tipos e funções listados. O truncamento de texto respeita fronteira de `char`. `human_size` usa base 1000 com 1 casa decimal (`B`, `KB`, `MB`, `GB`). O `Debug` de `ConnectTarget` mostra `password: "***"`.
- [ ] **Step 4:** `cargo test -p crustdb-core`. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): domain model, driver traits and cell formatting`.

### Task 3: Split de statements

**Files:** Create: `crates/crustdb-core/src/sql/mod.rs`, `sql/split.rs`; `crates/crustdb-core/benches/sql.rs`

**Interfaces:**
- Produces: `struct StatementSpan { range: Range<usize>, text: String }`, `fn split(text: &str, kind: DriverKind) -> Vec<StatementSpan>`, `fn statement_at(spans: &[StatementSpan], byte_offset: usize) -> Option<&StatementSpan>`, `fn statements_in(spans: &[StatementSpan], selection: Range<usize>) -> Vec<&StatementSpan>`

- [ ] **Step 1: Testes que falham:**
```rust
#[test] fn splits_on_semicolons() { let s = split("SELECT 1; SELECT 2;", DriverKind::Postgres); assert_eq!(texts(&s), ["SELECT 1", "SELECT 2"]); }
#[test] fn ignores_semicolons_in_strings_and_comments() { let s = split("SELECT ';' -- ;\n; /* ; */ SELECT 2", DriverKind::Mysql); assert_eq!(s.len(), 2); }
#[test] fn handles_pg_dollar_quotes() { let s = split("CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END $$ LANGUAGE plpgsql; SELECT 1", DriverKind::Postgres); assert_eq!(s.len(), 2); }
#[test] fn handles_mysql_delimiter() { let s = split("DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;", DriverKind::Mysql); assert_eq!(texts(&s), ["CREATE PROCEDURE p() BEGIN SELECT 1; END", "SELECT 2"]); }
#[test] fn statement_at_cursor_after_semicolon_picks_previous() { let t = "SELECT 1;   \nSELECT 2"; let s = split(t, DriverKind::Postgres); assert_eq!(statement_at(&s, 10).unwrap().text, "SELECT 1"); }
#[test] fn statement_at_with_multibyte_text() { let t = "SELECT 'ação';\nSELECT '🦀'"; let s = split(t, DriverKind::Postgres); let off = t.find("🦀").unwrap(); assert_eq!(statement_at(&s, off).unwrap().text, "SELECT '🦀'"); }
#[test] fn tokenizer_error_returns_whole_text() { let s = split("SELECT 'unterminated; SELECT 2", DriverKind::Postgres); assert_eq!(s.len(), 1); }
#[test] fn range_maps_back_to_source() { let t = "  SELECT 1 ;"; let s = split(t, DriverKind::Mysql); assert_eq!(&t[s[0].range.clone()], "SELECT 1"); }
```
- [ ] **Step 2:** `cargo test -p crustdb-core sql::split`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com o tokenizer do `sqlparser` (`PostgreSqlDialect`/`MySqlDialect`), convertendo `Location` (linha/coluna) em offset de byte por uma tabela de inícios de linha. Linhas `DELIMITER x` (MySQL, case-insensitive, início de linha) trocam o terminador da região seguinte e são removidas. `text` é o trecho aparado, sem terminador.
- [ ] **Step 4:** `cargo test -p crustdb-core sql::split`. Esperado: PASS. Bench `split_5000_lines` em `benches/sql.rs` (`cargo bench -p crustdb-core --bench sql -- split` executa).
- [ ] **Step 5:** Commit (com aprovação): `feat(core): statement splitter with cursor lookup`.

### Task 4: Classificação de statements

**Files:** Create: `crates/crustdb-core/src/sql/classify.rs`

**Interfaces:**
- Produces: `enum StatementClass { Read, Session, Tx(TxOp), CreateSafe, Dml(DmlKind), Destructive(DestructiveKind), Unknown }`; `enum TxOp { Begin, Commit, Rollback, Savepoint }`; `enum DmlKind { Insert, Update, Delete, Merge, Replace, LoadData, Copy }`; `enum DestructiveKind { DropTable, DropOther, Truncate, Alter, Rename, CreateOrReplace, Grant, Revoke }`; `struct TargetName { qualifier: Option<String>, name: String }`; `struct Classified { class: StatementClass, has_where: bool, values_rows: Option<u64>, target: Option<TargetName>, cursorable: bool, is_ddl: bool }`; `fn classify(sql: &str, kind: DriverKind) -> Classified`; `impl Classified { fn exec_mode(&self) -> ExecMode }` (`Read` só se `cursorable`)

- [ ] **Step 1: Testes que falham** (tabela §4.7 da spec):
```rust
#[test] fn select_is_read() { let c = classify("SELECT * FROM t", DriverKind::Postgres); assert_eq!(c.class, StatementClass::Read); assert!(c.cursorable); }
#[test] fn cte_with_delete_is_dml() { assert!(matches!(classify("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d", DriverKind::Postgres).class, StatementClass::Dml(_))); }
#[test] fn explain_analyze_delete_is_dml() { assert!(matches!(classify("EXPLAIN ANALYZE DELETE FROM t", DriverKind::Postgres).class, StatementClass::Dml(DmlKind::Delete))); }
#[test] fn update_without_where_flagged() { let c = classify("UPDATE t SET a = 1", DriverKind::Mysql); assert!(!c.has_where); }
#[test] fn insert_values_counts_tuples() { assert_eq!(classify("INSERT INTO t VALUES (1),(2),(3)", DriverKind::Mysql).values_rows, Some(3)); }
#[test] fn drop_truncate_alter_are_destructive() { for s in ["DROP TABLE t", "TRUNCATE t", "ALTER TABLE t DROP COLUMN a", "DROP SCHEMA s CASCADE"] { assert!(matches!(classify(s, DriverKind::Postgres).class, StatementClass::Destructive(_)), "{s}"); } }
#[test] fn drop_table_has_target() { let c = classify("DROP TABLE public.users", DriverKind::Postgres); assert_eq!(c.target.unwrap().name, "users"); }
#[test] fn create_or_replace_is_destructive() { assert!(matches!(classify("CREATE OR REPLACE VIEW v AS SELECT 1", DriverKind::Postgres).class, StatementClass::Destructive(DestructiveKind::CreateOrReplace))); }
#[test] fn side_effect_function_is_unknown() { assert_eq!(classify("SELECT pg_terminate_backend(42)", DriverKind::Postgres).class, StatementClass::Unknown); }
#[test] fn unparseable_is_unknown() { assert_eq!(classify("FROBNICATE everything", DriverKind::Mysql).class, StatementClass::Unknown); }
#[test] fn begin_commit_are_tx() { assert_eq!(classify("START TRANSACTION", DriverKind::Mysql).class, StatementClass::Tx(TxOp::Begin)); }
```
- [ ] **Step 2:** `cargo test -p crustdb-core sql::classify`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com `sqlparser::parser::Parser::parse_sql` mais um visitor sobre o AST. A lista de funções com efeito colateral vem verbatim da spec §4.7 (comparação case-insensitive pelo nome sem schema). `is_ddl = true` para `CREATE`/`ALTER`/`DROP`/`TRUNCATE`/`RENAME` (usado na regra de commit implícito do MySQL, §4.6). Falha de parse → `Unknown`.
- [ ] **Step 4:** `cargo test -p crustdb-core sql::classify`. Esperado: PASS. Bench `classify_1000_statements`.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): AST-based statement classification`.

### Task 5: Política de segurança

**Files:** Create: `crates/crustdb-core/src/safety.rs`

**Interfaces:**
- Consumes: `Classified`, `StatementClass` (Task 4).
- Produces: `struct SafetyPolicy { threshold: u64 }`; `enum EstimateNeed { None, Explain, TableRows }`; `fn estimate_need(c: &Classified) -> EstimateNeed`; `struct Assessment { index: usize, sql: String, classified: Classified, estimate: Option<u64> }`; `enum Verdict { Run, Confirm, Password }` (`Ord`, Run < Confirm < Password); `enum FlagReason { NoWhere, MultiRow(u64), OverThreshold(u64), UnknownEstimate, Destructive(DestructiveKind), Unrecognized }`; `struct Flag { index: usize, verdict: Verdict, reason: FlagReason }`; `struct Evaluation { verdict: Verdict, flags: Vec<Flag> }`; `impl SafetyPolicy { fn evaluate(&self, items: &[Assessment]) -> Evaluation }`

- [ ] **Step 1: Testes que falham:**
```rust
#[test] fn dml_over_threshold_requires_password() { assert_eq!(eval("UPDATE t SET a=1 WHERE b>0", Some(1001), 1000).verdict, Verdict::Password); }
#[test] fn unknown_estimate_requires_password() { assert_eq!(eval("DELETE FROM t WHERE a=1", None, 1000).verdict, Verdict::Password); }
#[test] fn multi_row_requires_confirm() { assert_eq!(eval("DELETE FROM t WHERE a>1", Some(2), 1000).verdict, Verdict::Confirm); }
#[test] fn no_where_requires_confirm_even_if_small() { assert_eq!(eval("DELETE FROM t", Some(1), 1000).verdict, Verdict::Confirm); }
#[test] fn single_row_with_where_runs() { assert_eq!(eval("UPDATE t SET a=1 WHERE id=7", Some(1), 1000).verdict, Verdict::Run); }
#[test] fn insert_values_uses_tuple_count() { assert_eq!(estimate_need(&classify("INSERT INTO t VALUES (1)", DriverKind::Mysql)), EstimateNeed::None); }
#[test] fn destructive_requires_confirm() { assert_eq!(eval("ALTER TABLE t DROP COLUMN a", None, 1000).verdict, Verdict::Confirm); }
#[test] fn truncate_big_table_requires_password() { assert_eq!(eval("TRUNCATE t", Some(5000), 1000).verdict, Verdict::Password); }
#[test] fn unrecognized_requires_confirm() { assert_eq!(eval("CALL p()", None, 1000).verdict, Verdict::Confirm); }
#[test] fn batch_takes_strictest_verdict() { /* SELECT + DELETE sem WHERE + UPDATE com E=5000 */ let e = eval_batch(&[("SELECT 1", None), ("DELETE FROM a", Some(3)), ("UPDATE b SET x=1 WHERE y>0", Some(5000))], 1000); assert_eq!(e.verdict, Verdict::Password); assert_eq!(e.flags.len(), 2); }
#[test] fn reads_never_flagged() { assert!(eval("SELECT * FROM t", None, 1).flags.is_empty()); }
```
- [ ] **Step 2:** `cargo test -p crustdb-core safety`. Esperado: FAIL.
- [ ] **Step 3:** Implementar as regras da spec §4.7 e D22–D24 na ordem: Password (E > threshold ou E desconhecido em DML; `DropTable`/`Truncate` com E > threshold) → Confirm (sem `WHERE` em `UPDATE`/`DELETE`, E > 1, qualquer `Destructive`, `Unknown`) → Run. `estimate_need`: `Dml` com `values_rows` → `None`; demais `Dml` → `Explain`; `DropTable`/`Truncate` → `TableRows`; resto → `None`.
- [ ] **Step 4:** `cargo test -p crustdb-core safety`. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): safety policy with confirm and password verdicts`.

### Task 6: Query de tabela e trait `Dialect`

**Files:** Create: `crates/crustdb-core/src/query.rs`, `dialect.rs`

**Interfaces:**
- Produces (`query`): `enum Operator { Eq, Ne, Lt, Gt, Le, Ge, In, NotIn, Between, IsNull, IsNotNull, Contains, NotContains, StartsWith, EndsWith, Like, NotLike, ILike }` com `fn arity(&self) -> Arity { Zero, One, Two, Many }`; `struct Filter { column: String, op: Operator, values: Vec<String> }`; `struct Sort { column: String, descending: bool }`; `struct TableQuery { object: ObjectRef, columns: Vec<ColumnMeta>, filters: Vec<Filter>, sorts: Vec<Sort>, limit: Option<u64>, offset: u64 }`; `fn operators_for(cat: TypeCategory, kind: DriverKind) -> &'static [Operator]`; `fn parse_in_list(&str) -> Vec<String>`; `enum QueryError { UnknownColumn(String), Arity { op: Operator, got: usize }, OperatorNotAllowed(Operator) }`
- Produces (`dialect`): `trait Dialect: Send + Sync { fn kind(&self) -> DriverKind; fn quote_ident(&self, s: &str) -> String; fn qualified_name(&self, o: &ObjectRef) -> String; fn param(&self, index: usize, column: &ColumnMeta) -> String; fn select_expr(&self, column: &ColumnMeta) -> String; fn ci_like(&self, column: &ColumnMeta, param: &str) -> String; }`; `struct BuiltQuery { sql: String, params: Vec<Param> }`; `fn build_table_query(d: &dyn Dialect, q: &TableQuery) -> Result<BuiltQuery, QueryError>`; `fn escape_like(value: &str) -> String` (caractere de escape `!`: escapa `!`, `%`, `_`)

- [ ] **Step 1: Testes que falham**, com um `TestDialect` local (aspas `"`, `$n`) só no módulo de teste:
```rust
#[test] fn multi_sort_order_preserved() { let q = tq(vec![], vec![sort("b", true), sort("a", false)]); assert!(build(&q).sql.ends_with(r#"ORDER BY "b" DESC, "a" ASC LIMIT 301 OFFSET 0"#)); }
#[test] fn default_sort_uses_pk() { let q = tq_with_pk("id"); assert!(build(&q).sql.contains(r#"ORDER BY "id" ASC"#)); }
#[test] fn contains_escapes_wildcards() { assert_eq!(escape_like("50%_a!"), "50!%!_a!!"); }
#[test] fn in_list_binds_each_value() { let b = build(&tq(vec![filter("id", Operator::In, &["1", "2", "3"])], vec![])); assert_eq!(b.params.len(), 3); }
#[test] fn arity_mismatch_rejected() { assert!(matches!(build_err(filter("id", Operator::Between, &["1"])), QueryError::Arity { .. })); }
#[test] fn binary_allows_only_null_checks() { assert_eq!(operators_for(TypeCategory::Binary, DriverKind::Postgres), &[Operator::IsNull, Operator::IsNotNull]); }
#[test] fn ilike_only_on_postgres() { assert!(!operators_for(TypeCategory::Text, DriverKind::Mysql).contains(&Operator::ILike)); }
#[test] fn export_query_has_no_limit() { let mut q = tq(vec![], vec![]); q.limit = None; assert!(!build(&q).sql.contains("LIMIT")); }
```
- [ ] **Step 2:** `cargo test -p crustdb-core query dialect`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. A forma do SQL é `SELECT <select_expr…> FROM <qualified_name> [WHERE f1 AND f2…] [ORDER BY …] [LIMIT n OFFSET k]`. Os operadores seguem a spec §4.3. `contains`/`starts with`/`ends with` → `ci_like` com `%v%` / `v%` / `%v`, sobre `escape_like(v)`. `NOT contains` → `NOT (…)`. `IS NULL` não tem parâmetro. O chamador passa `limit = page_size + 1` (spec D15).
- [ ] **Step 4:** `cargo test -p crustdb-core`. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): table query builder and dialect trait`.

### Task 7: Dialetos PostgreSQL e MySQL

**Files:** Create: `crates/crustdb-postgres/src/dialect.rs`, `crates/crustdb-mysql/src/dialect.rs`

**Interfaces:**
- Consumes: `Dialect`, `build_table_query` (Task 6).
- Produces: `crustdb_postgres::PgDialect`, `crustdb_mysql::MysqlDialect` (structs unitárias que implementam `Dialect`).

- [ ] **Step 1: Testes golden que falham** (SQL e parâmetros exatos):
```rust
// PgDialect
#[test] fn quote_ident_doubles_quote_char() { assert_eq!(PgDialect.quote_ident(r#"we"ird Name"#), r#""we""ird Name""#); }
#[test] fn pg_param_casts_text_to_column_type() { assert_eq!(PgDialect.param(1, &col("id", "int4", TypeCategory::Integer)), "CAST($1 AS int4)"); }
#[test] fn pg_select_caps_text_transfer() { assert_eq!(PgDialect.select_expr(&col("body", "text", TypeCategory::Text)), r#"left("body"::text, 1048576) AS "body""#); }
#[test] fn pg_select_non_text_casts_to_text() { assert_eq!(PgDialect.select_expr(&col("n", "int4", TypeCategory::Integer)), r#""n"::text AS "n""#); }
#[test] fn pg_contains_uses_ilike() { assert_eq!(PgDialect.ci_like(&col("e", "text", TypeCategory::Text), "$1"), r#""e"::text ILIKE $1 ESCAPE '!'"#); }
// MysqlDialect
#[test] fn mysql_quote_ident_doubles_backtick() { assert_eq!(MysqlDialect.quote_ident("a`b"), "`a``b`"); }
#[test] fn mysql_param_is_question_mark() { assert_eq!(MysqlDialect.param(3, &col("id", "int", TypeCategory::Integer)), "?"); }
#[test] fn mysql_contains_lowercases_both_sides() { assert_eq!(MysqlDialect.ci_like(&col("e", "varchar(10)", TypeCategory::Text), "?"), "LOWER(CAST(`e` AS CHAR)) LIKE LOWER(?) ESCAPE '!'"); }
#[test] fn mysql_select_caps_text_and_binary() { assert_eq!(MysqlDialect.select_expr(&col("b", "blob", TypeCategory::Binary)), "LEFT(`b`, 1048576) AS `b`"); }
```
- [ ] **Step 2:** `cargo test -p crustdb-postgres -p crustdb-mysql dialect`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. PG: `select_expr` faz cast de toda coluna para `::text` (Integer, Decimal etc. só com cast; Text, Json, Binary e Other com `left(…, CELL_MEMORY_CAP)`). MySQL: `LEFT` só em Text, Json, Binary e Other; o resto vai sem cast (o statement preparado devolve valores tipados, tratados na Task 15).
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(drivers): postgres and mysql dialects`.

### Task 8: Parser de URL de conexão

**Files:** Create: `crates/crustdb-core/src/url.rs`

**Interfaces:**
- Produces: `struct ParsedUrl { driver: DriverKind, host: String, port: Option<u16>, user: Option<String>, password: Option<SecretString>, database: Option<String>, tls_mode: Option<TlsMode>, warnings: Vec<UrlWarning> }`; `enum UrlWarning { DowngradedToRequire(String) }`; `enum UrlError { UnsupportedScheme(String), MissingHost, InvalidPort, Malformed }`; `fn parse_connection_url(s: &str) -> Result<ParsedUrl, UrlError>`

- [ ] **Step 1: Testes que falham:** `parses_postgres_with_sslmode` (`postgres://u:p%40ss@h:6543/db?sslmode=verify-full` → user `u`, senha `p@ss` decodificada, porta 6543, `VerifyFull`); `postgresql_scheme_accepted`; `mariadb_maps_to_mysql`; `mysql_ssl_mode_verify_identity_maps_to_verify_full`; `prefer_downgrades_to_require_with_warning`; `unknown_scheme_rejected` (`mongodb://h`); `ipv6_host_parsed` (`postgres://[::1]:5432/db` → host `::1`).
- [ ] **Step 2:** `cargo test -p crustdb-core url`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com o crate `url`. Mapa de `sslmode` (PG) e `ssl-mode` (MySQL): `disable`/`DISABLED` → Disable; `allow`/`prefer`/`PREFERRED` → Require + aviso; `require`/`REQUIRED` → Require; `verify-ca`/`VERIFY_CA` → VerifyCa; `verify-full`/`VERIFY_IDENTITY` → VerifyFull.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): connection URL parsing`.

### Task 9: Realce e contexto de autocomplete

**Files:** Create: `crates/crustdb-core/src/sql/highlight.rs`, `sql/complete.rs`, `sql/keywords.rs`

**Interfaces:**
- Produces: `enum TokenKind { Keyword, String, Number, Comment, QuotedIdent, Ident, Operator, Punct, Whitespace }`; `fn highlight(text: &str, kind: DriverKind) -> Vec<(Range<usize>, TokenKind)>`; `enum Expect { Keyword, Table, Column { qualifier: Option<String> }, Any }`; `struct TableAlias { name: String, alias: Option<String> }`; `struct CompletionContext { prefix: String, prefix_range: Range<usize>, expect: Expect, referenced_tables: Vec<TableAlias> }`; `fn completion_context(text: &str, byte_offset: usize, kind: DriverKind) -> CompletionContext`; `fn rank(candidates: &[&str], prefix: &str, max: usize) -> Vec<usize>`; `fn keywords(kind: DriverKind) -> &'static [&'static str]`

- [ ] **Step 1: Testes que falham:** `highlight_covers_all_bytes` (os ranges cobrem `0..len` sem buracos); `highlight_marks_keyword_string_comment`; `highlight_unterminated_string_does_not_panic`; `after_from_expects_table`; `after_alias_dot_expects_column_with_qualifier` (`SELECT u. FROM users u`, cursor após `u.` → `Column { qualifier: Some("u") }`, `referenced_tables` contém `users` com alias `u`); `select_list_expects_column_without_qualifier`; `rank_prefix_before_substring` (`["user_id", "id", "users"]`, prefixo `us` → `[0, 2]`, depois as substrings); `rank_caps_at_max` (`max = 50`).
- [ ] **Step 2:** `cargo test -p crustdb-core sql::highlight sql::complete`. Esperado: FAIL.
- [ ] **Step 3:** Implementar sobre o mesmo tokenizer da Task 3 (reusar a conversão de `Location` em offset). `rank` é case-insensitive e mantém a ordem original dentro de cada grupo.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS. Bench `highlight_5000_lines`.
- [ ] **Step 5:** Commit (com aprovação): `feat(core): sql highlighting and completion context`. Abrir o PR da M1 com aprovação do usuário.

---

## Milestone M2: Drivers (branch `feat/m2-drivers`)

### Task 10: Testkit (containers e certificados)

**Files:** Create: `crates/crustdb-testkit/src/lib.rs`, `src/certs.rs`, `src/containers.rs`

**Interfaces:**
- Produces: `struct TestCerts { ca_pem: String, server_cert_pem: String, server_key_pem: String, dir: tempfile::TempDir }`; `fn certs_for(hostnames: &[&str]) -> TestCerts` (rcgen; CA própria); `async fn start_postgres(tls: Option<&TestCerts>) -> PgContainer { host, port, user, password, _guard }` (imagem de `CRUSTDB_PG_IMAGE`, padrão `postgres:latest`); `async fn start_mysql(tls: Option<&TestCerts>) -> MysqlContainer` (imagem de `CRUSTDB_MYSQL_IMAGE`, padrão `mysql:8.4`); `async fn start_sshd(network) -> SshContainer` (`linuxserver/openssh-server`, senha + chave + `known_hosts`).

- [ ] **Step 1:** Escrever o teste `containers_start_and_accept_connections` em `crates/crustdb-testkit/tests/smoke.rs` (TCP connect na porta mapeada de cada container). Marcar com `#[ignore = "docker"]`; o CI roda com `-- --include-ignored`.
- [ ] **Step 2:** `cargo test -p crustdb-testkit -- --include-ignored`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com `testcontainers` (`GenericImage`), montando os certificados no container. PG: `ssl=on`, `ssl_cert_file`, `ssl_key_file`. MySQL: `--ssl-ca --ssl-cert --ssl-key`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS. Ligar o job `integration` do CI com a matriz da spec §8 (`postgres:12`, `postgres:latest`, `mysql:5.6`, `mysql:5.7`, `mysql:8.4`, `mariadb:10.0` ou a menor tag disponível, `mariadb:latest`).
- [ ] **Step 5:** Commit (com aprovação): `test: container and certificate testkit`.

### Task 11: PostgreSQL: conexão, TLS e ping

**Files:** Create: `crates/crustdb-postgres/src/connect.rs`, `src/session.rs` (esqueleto), `tests/integration.rs`; Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `ConnectTarget`, `SessionPurpose`, `TlsConfig` (Task 2); testkit (Task 10).
- Produces: `struct PostgresDriver` (`impl Driver`); `PgSession` com `server_version` (`Product::Postgres`, a partir de `server_version`), `ping` e `cancel_handle` (`CancelToken`).

- [ ] **Step 1: Testes de integração que falham:** `connects_with_each_tls_mode` (os quatro modos contra um servidor com cert válido para `localhost`); `verify_full_rejects_wrong_hostname` (cert só para `other.test` → `ErrorKind::Tls`); `verify_ca_accepts_wrong_hostname`; `disable_against_tls_required_server_fails_cleanly`; `metadata_session_rejects_writes` (`CREATE TABLE x()` numa sessão `Metadata` → `ErrorKind::Query` com código `25006`); `auth_failure_maps_to_auth_kind`; `connect_timeout_after_10s` (IP não roteável, `connect_timeout` curto no teste); `hostaddr_used_with_real_tls_name` (`connect_addr = 127.0.0.1:porta`, `host = "localhost"`, `verify-full` passa).
- [ ] **Step 2:** `cargo test -p crustdb-postgres --test integration -- --include-ignored`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com `tokio_postgres::Config` (`host` = host real, `hostaddr`/porta de `connect_addr`, `connect_timeout`) e `tokio-postgres-rustls`, usando um `rustls::ClientConfig` por modo: `Require` = verifier que aceita qualquer cert; `VerifyCa` = verifier que valida a cadeia e ignora o nome; `VerifyFull` = `WebPkiServerVerifier` padrão. Raízes = `ca_path` ou `rustls-native-certs`. A task de `Connection` é spawnada. Em sessões read-only, rodar `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` logo após conectar.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(postgres): connect with tls modes and read-only sessions`.

### Task 12: PostgreSQL: execução, cursor, cancel e EXPLAIN

**Files:** Modify: `crates/crustdb-postgres/src/session.rs`; Create: `src/types.rs`; Test: `tests/integration.rs`

**Interfaces:**
- Consumes: `Cell::*_capped` (Task 2). O chamador decide o `ExecMode`.
- Produces: implementação completa de `Session` para PG (D27, D29, D37, D38); `fn category_for_oid(oid: u32) -> TypeCategory` em `types.rs`.

- [ ] **Step 1: Testes de integração que falham:**
  - `read_cursor_load_more_bounded_memory`: `start("SELECT g FROM generate_series(1, 50000000) g", ExecMode::Read)`, `fetch(1000)` duas vezes, linhas `"1"` e `"1001"`; RSS do processo de teste cresce < 50 MB.
  - `zero_row_select_keeps_columns`: `SELECT 1 AS a WHERE false` → header com coluna `a`, `exhausted = true`.
  - `pg_editor_columns_have_type_categories`: `SELECT 1::int4 a, '{}'::jsonb b, '\x01'::bytea c` → categorias Integer, Json, Binary; `c` vira `Cell::Bytes`.
  - `write_with_returning_is_drained_not_cancelled`: `INSERT … SELECT generate_series(1, 5000) RETURNING id` com `fetch(10)` e depois `finish()` → 5.000 linhas existem na tabela.
  - `paused_cursor_finished_before_next_command`: um `start` com cursor pausado, seguido de outro `start`, funciona, e `pg_stat_activity` não mostra a sessão `idle in transaction`.
  - `cursor_inside_user_transaction_keeps_it_open`: `BEGIN` → `start(SELECT…)` → `finish` → a transação continua aberta.
  - `cancel_long_query`: `SELECT pg_sleep(60)` cancelado → `ErrorKind::Cancelled` em ≤ 1 s.
  - `explain_rows_for_update_delete`: tabela com 10.000 linhas analisada; `explain_rows("UPDATE t SET a=a WHERE id <= 5000")` dá `Some(n)` com n entre 2.500 e 10.000.
  - `pg_aborted_transaction_state`: erro dentro de `BEGIN` → o próximo statement falha com SQLSTATE `25P02`.
  - `grid_caps_cell_transfer_at_1mib`: `start_params` com o SQL da Task 7 sobre uma linha de 50 MB → `Cell::Text { truncated: true }` com 1 MiB.
  - `server_restart_during_paused_cursor_reports_error`: o container reinicia com o cursor pausado → `fetch` devolve `ErrorKind::Connect`, sem panic.
- [ ] **Step 2:** Mesmo comando da Task 11. Esperado: FAIL.
- [ ] **Step 3:** Implementar:
  - Tipos via `client.prepare(sql)` (D38); falhou → `Other`.
  - `ExecMode::Read`: `BEGIN` (só se não houver transação do usuário, rastreada pela sessão via os `TxOp` que passam por ela), `DECLARE crustdb_cur NO SCROLL CURSOR FOR <sql>`; `fetch(n)` = `FETCH n FROM crustdb_cur` por `simple_query`; `finish` = `CLOSE` + `COMMIT` da transação implícita.
  - `ExecMode::Other`: `simple_query_raw`, guardando até o limite pedido em `fetch` e drenando o resto em `finish`.
  - `start_params`: `query_typed_raw` com `Type::TEXT`.
  - `bytea` (`\x…`) é decodificado para `Cell::Bytes`.
  - `explain_rows`: `EXPLAIN (FORMAT JSON)`, pegando `Plan Rows` do filho do nó `ModifyTable`.
  - O timer de 5 min de ociosidade fica no ator (Task 23), não aqui.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(postgres): text execution with server cursors, cancel and explain`.

### Task 13: PostgreSQL: introspecção e página do grid

**Files:** Create: `crates/crustdb-postgres/src/introspect.rs`; Test: `tests/integration.rs`

**Interfaces:**
- Consumes: `Introspector`, `PgDialect`, `build_table_query`, `category_for_oid` (Task 12).
- Produces: `PgIntrospector` (`impl Introspector`), devolvido por `PostgresDriver::introspector()`.

- [ ] **Step 1: Testes de integração que falham**, com uma fixture SQL que cria schemas `public` e `app`, tabela particionada, view, matview, função, procedure, trigger, sequence, enum, domain, composite, FKs e índices:
  - `lists_tree_levels_per_driver`: todas as pastas da spec §4.2 têm o objeto da fixture; `pg_toast` não aparece.
  - `row_estimate_without_count`: após `ANALYZE`, `row_estimate` ≈ linhas; tabela nunca analisada → `None` (reltuples `-1`).
  - `ddl_for_each_object_kind`: DDL não vazio e contendo o nome de cada objeto.
  - `structure_lists_columns_indexes_fks`.
  - `fetches_page_with_filters_and_sorts`: `=` em int, `contains` com `%` literal, `IN`, `BETWEEN`, `IS NULL`, sort múltiplo; páginas 1 e 2 sem sobreposição.
  - `filter_value_injection_is_inert`: valor `'; DROP TABLE t; --` → zero linhas e a tabela continua existindo.
  - `table_with_quoted_mixed_case_name_opens`: tabela `"Weird ""Name"""` em schema `"Ação"`.
- [ ] **Step 2:** Mesmo comando. Esperado: FAIL.
- [ ] **Step 3:** Implementar as queries de catálogo da spec §4.2/§4.3. Todos os filtros de nome são parâmetros bound.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(postgres): catalog introspection and grid pages`.

### Task 14: MySQL: conexão, TLS e ping

**Files:** Create: `crates/crustdb-mysql/src/connect.rs`, `src/session.rs` (esqueleto), `tests/integration.rs`; Modify: `src/lib.rs`

**Interfaces:**
- Produces: `struct MysqlDriver` (`impl Driver`); `MysqlSession` com `Product::Mysql` ou `Product::Mariadb` (detectado em `SELECT VERSION()`), `ping` e `cancel_handle` (conexão efêmera com `KILL QUERY <connection_id>`).

- [ ] **Step 1: Testes que falham:** os mesmos cenários da Task 11 adaptados (`connects_with_each_tls_mode`, `verify_full_rejects_wrong_hostname`, `verify_ca_accepts_wrong_hostname`, `metadata_session_rejects_writes` com erro `1792`, `auth_failure_maps_to_auth_kind`, `hostname_override_used_with_tunnel_addr`), mais `tls_rejects_server_without_tls12` (só na imagem `mysql:5.6`; espera a mensagem da D39) e `detects_mariadb_product`.
- [ ] **Step 2:** `cargo test -p crustdb-mysql --test integration -- --include-ignored`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com `OptsBuilder`, sem handler de `LOCAL INFILE`. `SslOpts` por modo:
  - `Require`: `with_danger_accept_invalid_certs(true)`.
  - `VerifyCa`: `with_danger_skip_domain_validation(true)`.
  - `VerifyFull`: `with_danger_tls_hostname_override(Some(host real))` quando houver `connect_addr`.
  - Raiz extra: `with_root_certs`.
  - Sessões read-only: `SET SESSION TRANSACTION READ ONLY`.
  - Falha de handshake TLS por versão do protocolo vira a mensagem da D39.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(mysql): connect with tls modes and read-only sessions`.

### Task 15: MySQL: execução, cursor, cancel e EXPLAIN

**Files:** Modify: `crates/crustdb-mysql/src/session.rs`; Create: `src/conn_task.rs`, `src/types.rs`; Test: `tests/integration.rs`

**Interfaces:**
- Produces: `Session` para MySQL. `MysqlSession` é um handle (`mpsc::Sender<ConnCmd>`) para uma task dona da `Conn` (`conn_task.rs`), que mantém o `QueryResult` pausado entre comandos (spec §5). Também `fn category_for_column(col: &mysql_async::Column) -> TypeCategory` e `fn value_to_cell(v: mysql_async::Value, col: &ColumnMeta, binary_charset: bool) -> Cell`.

- [ ] **Step 1: Testes que falham:**
  - Mesmos cenários da Task 12: `read_cursor_load_more_bounded_memory` com tabela de 5M linhas gerada no setup, `zero_row_select_keeps_columns`, `write_is_drained_not_cancelled`, `paused_cursor_finished_before_next_command`, `cancel_long_query` (`SELECT SLEEP(60)`), `explain_rows_for_update_delete`, `server_restart_during_paused_cursor_reports_error`.
  - `transaction_states_and_implicit_commit_mysql`: `BEGIN`, `INSERT`, `CREATE TABLE`, `ROLLBACK` → o `INSERT` persiste.
  - `prepared_datetime_formats` (§4.4: `DATE`, `DATETIME(6)`, `TIME` negativo).
  - `latin1_column_displays`: coluna latin1 com `é` exibe `é`.
- [ ] **Step 2:** Mesmo comando da Task 14. Esperado: FAIL.
- [ ] **Step 3:** Implementar:
  - `ExecMode::Read`: `query_iter` pausado; `finish` faz `KILL QUERY` pela conexão efêmera e drena.
  - `ExecMode::Other`: drenar tudo, guardando até o limite.
  - `start_params`: `exec_iter`.
  - Colunas com charset 63 → `Cell::Bytes`; demais → `Cell::text_lossy_capped`.
  - `explain_rows`: maior `rows` entre as linhas do `EXPLAIN` cuja `table` é a tabela alvo.
  - `use_database`: `USE`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(mysql): owned-connection task, paused results, cancel and explain`.

### Task 16: MySQL: introspecção e página do grid

**Files:** Create: `crates/crustdb-mysql/src/introspect.rs`; Test: `tests/integration.rs`

**Interfaces:** Produces: `MysqlIntrospector` (`impl Introspector`).

- [ ] **Step 1: Testes que falham:** `lists_tree_levels_per_driver` (Tables, Views, Functions, Procedures, Triggers), `row_estimate_without_count` (`TABLE_ROWS`; view → `None`), `ddl_for_each_object_kind` (`SHOW CREATE …`), `structure_lists_columns_indexes_fks`, `fetches_page_with_filters_and_sorts`, `filter_value_injection_is_inert`, `table_with_quoted_mixed_case_name_opens` (`` `we``ird` ``), `contains_is_case_insensitive_under_binary_collation`.
- [ ] **Step 2:** Mesmo comando da Task 14. Esperado: FAIL.
- [ ] **Step 3:** Implementar via `information_schema` com parâmetros bound. O DDL usa `SHOW CREATE` com identificador citado.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS. Rodar a matriz completa no CI. Se `mysql:5.6` ou `mariadb:10.0` falhar por limitação do `mysql_async`, subir o piso (D6) e registrar no README.
- [ ] **Step 5:** Commit (com aprovação): `feat(mysql): information_schema introspection and grid pages`. Abrir o PR da M2.

---

## Milestone M3: App infra (branch `feat/m3-app-infra`)

### Task 17: Caminhos XDG e config de conexões

**Files:** Create: `crates/crustdb-app/src/storage/mod.rs`, `storage/paths.rs`, `storage/config.rs`

**Interfaces:**
- Produces: `fn config_dir() -> PathBuf`, `fn data_dir() -> PathBuf`, `fn state_dir() -> PathBuf` (`$XDG_*` ou o fallback em `$HOME`; criados com 0700); `struct ConnectionConfig { id: ConnectionId, name: String, driver: DriverKind, host: String, port: u16, user: String, database: Option<String>, save_password: bool, tls: TlsConfig, ssh: Option<SshConfig>, page_size: u32, editor_row_limit: u32, safety_threshold: u64 }`; `struct SshConfig { host: String, port: u16, user: String, auth: SshAuth }`; `enum SshAuth { Password, Key { key_path: PathBuf }, Agent }` (serde `kind`); `struct ConfigStore { path: PathBuf }` com `fn load(&self) -> Result<LoadedConfig, ConfigError>` e `fn save(&self, c: &[ConnectionConfig]) -> Result<(), ConfigError>`; `struct LoadedConfig { connections: Vec<ConnectionConfig>, read_only: bool, warning: Option<String> }`; `fn validate(c: &ConnectionConfig, others: &[ConnectionConfig]) -> Result<(), Vec<FieldError>>`; `struct FieldError { field: &'static str, message: String }`

- [ ] **Step 1: Testes que falham:** `config_roundtrip_matches_spec_json` (o JSON da spec §6 desserializa e serializa de volta, e os nomes de campo batem); `config_roundtrip_has_no_secrets` (nenhuma chave `password`); `config_written_atomically_with_0600` (arquivo temporário + rename; `mode & 0o777 == 0o600`); `unknown_version_opens_read_only`; `invalid_json_backed_up_and_empty`; `validate_rejects_duplicate_name_and_bad_ranges` (`port 0`, `page_size 5`, `editor_row_limit 200000`, `safety_threshold 0`).
- [ ] **Step 2:** `cargo test -p crustdb-app storage::config`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. Com `read_only = true`, `save` devolve `ConfigError::ReadOnly`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): xdg paths and connection config store`.

### Task 18: Segredos e histórico

**Files:** Create: `crates/crustdb-app/src/storage/secrets.rs`, `storage/history.rs`

**Interfaces:**
- Produces: `trait SecretStore: Send + Sync { fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError>; fn set(&self, account: &str, v: &SecretString) -> Result<(), SecretError>; fn delete(&self, account: &str) -> Result<(), SecretError>; fn available(&self) -> bool; }`, com `KeyringStore` (service `crustdb`), `MemoryStore` (testes) e `UnavailableStore` (sem Secret Service: `get` → `None`, `set` → erro `Unavailable`); `fn detect_secret_store() -> Box<dyn SecretStore>`; `fn account(id: ConnectionId, kind: SecretKind) -> String`, com `enum SecretKind { Db, SshPassword, SshPassphrase }` → `<uuid>:db`, `<uuid>:ssh-password`, `<uuid>:ssh-passphrase`.
- Produces: `struct HistoryEntry { ts: String, database: Option<String>, sql: String, duration_ms: u64, rows: Option<u64>, ok: bool, error: Option<String> }`; `struct HistoryStore { dir: PathBuf }` com `fn append(&self, id: ConnectionId, e: &HistoryEntry) -> io::Result<()>`, `fn load(&self, id: ConnectionId) -> io::Result<Vec<HistoryEntry>>` (compacta para as 10.000 mais recentes ao abrir) e `fn search<'a>(entries: &'a [HistoryEntry], q: &str) -> Vec<&'a HistoryEntry>`.

- [ ] **Step 1: Testes que falham:** `account_names_match_spec`; `unavailable_store_never_persists`; `history_appends_jsonl_with_0600`; `history_compacts_to_limit` (10.050 linhas → 10.000, as mais recentes); `history_search_substring_case_insensitive`; `history_skips_corrupt_lines`. O teste `keyring_roundtrip` fica com `#[ignore = "needs secret service"]`.
- [ ] **Step 2:** `cargo test -p crustdb-app storage`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. `ts` em ISO-8601 UTC (`2026-10-17T14:03:11Z`), via `std::time::SystemTime` mais uma função de formatação pequena com teste próprio. Sem crate de datas.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): secret store and query history`.

### Task 19: Túnel SSH e known_hosts

**Files:** Create: `crates/crustdb-app/src/net/mod.rs`, `net/ssh.rs`, `net/known_hosts.rs`; Test: `crates/crustdb-app/tests/ssh.rs`

**Interfaces:**
- Produces: `struct KnownHosts { app_file: PathBuf, user_file: PathBuf }` com `fn check(&self, host: &str, port: u16, key: &russh::keys::PublicKey) -> HostKeyStatus` e `fn add(&self, host: &str, port: u16, key: &PublicKey) -> io::Result<()>`; `enum HostKeyStatus { Known, Unknown { fingerprint_sha256: String }, Mismatch }`; `struct SshSecrets { password: Option<SecretString>, passphrase: Option<SecretString> }`; `struct Tunnel { local_addr: SocketAddr }` (fecha no `Drop`); `async fn open_tunnel(cfg: &SshConfig, secrets: SshSecrets, target: (String, u16), known: &KnownHosts, ask: HostKeyAsk) -> Result<Tunnel, SshError>`, com `type HostKeyAsk = Box<dyn FnOnce(String) -> BoxFuture<'static, bool> + Send>`; `enum SshError { Connect, Auth, HostKeyRejected, HostKeyMismatch, Timeout, Io(String) }`

- [ ] **Step 1: Testes que falham:** `known_hosts_parses_openssh_format` (unit); `mismatch_detected` (unit); `ssh_tunnel_with_key_password_agent` (integração: sshd + postgres na mesma rede, tunnel → `PostgresDriver::connect` com `connect_addr = local_addr` e `verify-full` → `ping` ok; repetir para senha, chave com passphrase e agent via `SSH_AUTH_SOCK` de um `ssh-agent` iniciado no teste); `unknown_host_key_rejected` (`ask` devolve `false` → `HostKeyRejected`, nada gravado); `accepted_key_saved_to_app_file`; `verify_full_via_ssh_checks_real_hostname`.
- [ ] **Step 2:** `cargo test -p crustdb-app --test ssh -- --include-ignored`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com `russh` (`client::Handler::check_server_key` consulta `KnownHosts` e chama `ask` em `Unknown`). O listener fica em `127.0.0.1:0` e cada conexão aceita abre um canal `direct-tcpip`. Timeout de 10 s.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): ssh tunnel with strict host key checking`.

### Task 20: Métricas do processo e escritor CSV

**Files:** Create: `crates/crustdb-app/src/metrics.rs`, `src/csv_out.rs`

**Interfaces:**
- Produces: `fn parse_stat_ticks(stat: &str) -> Option<u64>` (utime + stime, campos 14 e 15, contados depois do `)` final), `fn parse_vm_rss_kb(status: &str) -> Option<u64>`, `struct ProcSampler` com `fn sample(&mut self) -> Option<ProcStats>`, `struct ProcStats { cpu_percent: f32, rss_bytes: u64 }` (`USER_HZ = 100`, D31).
- Produces: `struct CsvOut<W: Write> { inner: BufWriter<W> }` com `fn header(&mut self, cols: &[ColumnMeta]) -> io::Result<()>`, `fn row(&mut self, cells: &[Cell]) -> io::Result<()>` e `fn finish(self) -> io::Result<()>`. Escritor próprio (o crate `csv` não distingue `NULL` de string vazia); substitui o `csv` da stack da spec.

- [ ] **Step 1: Testes que falham:** `parses_stat_with_spaces_in_comm` (`comm` = `(my (proc))`); `parses_vm_rss`; `cpu_percent_from_two_samples`; `csv_null_is_empty_unquoted_and_empty_string_quoted` (`[Null, Text(""), Text("a")]` → `,"",a\n`); `csv_quotes_comma_quote_newline` (`a,"b` → `"a,""b"`); `csv_binary_as_0x_hex` (`[0xde, 0xad]` → `0xdead`).
- [ ] **Step 2:** `cargo test -p crustdb-app metrics csv_out`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. Fim de linha `\n`, UTF-8, separador `,`. Aspas quando o valor contém `,`, `"`, `\n` ou `\r`, ou é string vazia.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): process metrics and csv writer`.

### Task 21: Runtime, protocolo Command/Event e driver fake

**Files:** Create: `crates/crustdb-app/src/runtime.rs`, `crates/crustdb-app/tests/support/fake_driver.rs`

**Interfaces:**
- Produces: `struct RequestId(pub u64)`, `struct WorkspaceId(pub u64)`, `struct TabId(pub u64)`, todos `Copy + Eq + Hash`; `enum TxState { Idle, Open, Aborted }`; `struct Runtime` com `fn start(repaint: Arc<dyn Fn() + Send + Sync>, drivers: DriverRegistry) -> (Runtime, std::sync::mpsc::Receiver<Event>)`, `fn send(&self, cmd: Command)`, `fn next_request_id(&self) -> RequestId` e `fn shutdown(self)`; `struct DriverRegistry` (`DriverKind → Arc<dyn Driver>`); `struct ResolvedSecrets { db: Option<SecretString>, ssh: SshSecrets }` (`SshSecrets` da Task 19).
- Produces `enum Command` (variantes mínimas; campos podem crescer, nomes não mudam): `Connect { ws, config: ConnectionConfig, secrets: ResolvedSecrets }`, `Disconnect { ws }`, `HostKeyDecision { ws, accept: bool }`, `LoadChildren { ws, req, database: String, schema: Option<String>, kind: Option<ObjectKind> }`, `OpenTable { ws, tab, object: ObjectRef }`, `FetchPage { tab, req, query: TableQuery }`, `LoadStructure { tab, req }`, `LoadDdl { tab, req }`, `OpenEditor { ws, tab, database: Option<String> }`, `Evaluate { tab, req, statements: Vec<String> }`, `Execute { tab, req, evaluation: RequestId }`, `LoadMore { tab, req }`, `Tx { tab, op: TxOp }`, `UseDatabase { tab, database: String }`, `CancelTab { tab }`, `CloseTab { tab, resolve: Option<TxOp> }`, `ExportTable { tab, req, query: TableQuery, path: PathBuf }`, `ExportResult { tab, req, path: PathBuf }`, `CancelExport { req }`.
- Produces `enum Event`: `Connected { ws, server: ServerVersion }`, `ConnectFailed { ws, error: String }`, `HostKeyPrompt { ws, host: String, fingerprint: String }`, `Children { req, items: Vec<ObjectRef> }`, `Page { req, header: ResultHeader, rows: Vec<Row>, has_more: bool, estimate: Option<u64> }`, `Structure { req, columns: Vec<ColumnDetail>, indexes: Vec<IndexInfo>, fks: Vec<ForeignKeyInfo> }`, `Ddl { req, text: String }`, `Evaluated { req, evaluation: Evaluation }`, `StatementDone { req, index: usize, completion: Completion, duration: Duration, history: HistoryEntry }`, `ResultStarted { req, header: ResultHeader }`, `Rows { req, rows: Vec<Row>, exhausted: bool }`, `ExecError { req, index: usize, error: DriverError }`, `TxChanged { tab, state: TxState }`, `Ping { ws, latency: Duration }`, `ExportProgress { req, rows: u64 }`, `ExportDone { req, result: Result<u64, String> }`, `Notice { tab, text: String }`, `ActorCrashed { tab }`.
- Produces `FakeDriver`: implementa `Driver`/`Session`/`Introspector` em memória, com respostas programáveis (linhas, erros, atrasos, estimativas).

- [ ] **Step 1: Testes que falham:** `request_ids_are_monotonic`; `events_wake_ui_via_repaint` (o callback é chamado a cada evento); `rows_batched_at_most_500` (o fake devolve 1.200 linhas → eventos de 500, 500 e 200); `shutdown_joins_runtime_thread`.
- [ ] **Step 2:** `cargo test -p crustdb-app runtime`. Esperado: FAIL.
- [ ] **Step 3:** Implementar uma thread dedicada com `tokio::runtime::Builder::new_multi_thread().worker_threads(2)`. O roteador de comandos despacha para os atores por `ws`/`tab`, e um ator inexistente gera um `Event::Notice`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): tokio runtime bridge and command/event protocol`.

### Task 22: WorkspaceActor e DataActor

**Files:** Create: `crates/crustdb-app/src/actors/mod.rs`, `actors/workspace.rs`, `actors/data.rs`

**Interfaces:**
- Consumes: `Runtime`, `Command`/`Event` (Task 21); `open_tunnel` (Task 19); `Introspector`, `build_table_query`.
- Produces: atores que atendem `Connect`, `Disconnect`, `HostKeyDecision`, `LoadChildren`, `OpenTable`, `FetchPage`, `LoadStructure`, `LoadDdl`, `CancelTab` (para abas de tabela) e o ping de 60 s.

- [ ] **Step 1: Testes que falham** (com `FakeDriver`): `connect_opens_metadata_session_with_metadata_purpose`; `pg_second_database_opens_second_metadata_session`; `page_returns_has_more_only_when_limit_plus_one`; `page_attaches_estimate_only_without_filters`; `stale_request_id_ignored` (dois `FetchPage`; só o mais recente gera `Page` útil, e o anterior é cancelado); `cancel_tab_cancels_running_page`; `ping_every_60s_emits_latency` (`tokio::time::pause`); `metadata_reconnects_with_backoff` (1, 2, 4, 8 s, teto de 30 s); `actor_panic_emits_actor_crashed_and_restarts`.
- [ ] **Step 2:** `cargo test -p crustdb-app actors`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. O `DataActor` é criado em `OpenTable`, abre uma sessão `Data` só no primeiro `FetchPage` e é dono dela. O `WorkspaceActor` guarda o `Tunnel` e as sessões `Metadata` num `HashMap<String, Box<dyn Session>>` por database.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): workspace and data actors`.

### Task 23: EditorActor (execução, segurança, transação)

**Files:** Create: `crates/crustdb-app/src/actors/editor.rs`

**Interfaces:**
- Consumes: `split`, `classify`, `estimate_need`, `SafetyPolicy`, `Session` (Tasks 3–5, 2).
- Produces: um ator por aba de query, atendendo `OpenEditor`, `Evaluate`, `Execute`, `LoadMore`, `Tx`, `UseDatabase`, `CancelTab` e `CloseTab`; timer de 5 min do cursor pausado (D37).

- [ ] **Step 1: Testes que falham** (com `FakeDriver`):
  - `evaluate_finishes_paused_cursor_before_explain`.
  - `execute_rejects_unknown_evaluation_id`: `Execute` sem `Evaluate` prévio para os mesmos statements → `Notice`, nada executa.
  - `confirm_flow_cancel_executes_nothing`: `Evaluate` sem `Execute` não chega ao fake.
  - `run_all_stops_at_first_error`.
  - `only_last_statement_keeps_cursor`: `SELECT a; SELECT b` → o primeiro é lido até o limite e fechado; o segundo fica pausado.
  - `load_more_fetches_editor_row_limit`.
  - `idle_paused_cursor_auto_closed` (5 min com `tokio::time::pause`, mais um `Notice`).
  - `tx_state_follows_buttons_and_statements`: `Begin` → Open; erro no PG → Aborted; `ROLLBACK` → Idle; DDL no MySQL → Idle + `Notice` "implicit commit".
  - `use_database_blocked_with_open_tx_on_pg`.
  - `close_tab_with_open_tx_requires_resolution`.
  - `history_entry_emitted_per_statement`.
  - `editor_connection_lost_reports_and_reconnects_on_next_run`: o fake derruba a sessão → `Notice` "Connection lost — open transaction was rolled back by the server", `TxChanged` Idle, e o próximo `Execute` reconecta (spec §4.1).
- [ ] **Step 2:** `cargo test -p crustdb-app actors::editor`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. `Evaluate` monta as `Assessment`s (rodando `explain_rows`/`row_estimate` conforme `estimate_need`), guarda `(req, statements, evaluation)` e emite `Evaluated`. `Execute` só aceita o `req` guardado. A verificação de senha acontece na UI (Task 33): o ator confia que `Execute` só chega depois da aprovação, e o id amarra o lote avaliado ao lote executado.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(app): editor actor with safety evaluation and transactions`.

### Task 24: Export CSV

**Files:** Create: `crates/crustdb-app/src/actors/export.rs`; Modify: `actors/editor.rs`

**Interfaces:**
- Consumes: `CsvOut` (Task 20); `build_table_query` com `limit: None`.
- Produces: atende `ExportTable` (task própria com sessão `Export`), `ExportResult` (dentro do `EditorActor`, D26) e `CancelExport`.

- [ ] **Step 1: Testes que falham:** `export_full_table_streams_csv` (fake com 250.000 linhas → arquivo com 250.001 linhas e `ExportProgress` periódico); `export_respects_filters_and_sorts` (o SQL recebido pelo fake não tem `LIMIT`); `export_result_drains_without_reexecuting` (o fake conta 1 execução); `export_result_warns_on_truncated_cells`; `cancel_export_removes_partial_file`; `export_uses_export_purpose_session`.
- [ ] **Step 2:** `cargo test -p crustdb-app actors::export`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. Busca em lotes de 5.000 linhas, com progresso a cada lote.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS. Teste de integração `export_full_table_streams_csv` também contra PG e MySQL reais (em `crates/crustdb-app/tests/export_it.rs`, `#[ignore = "docker"]`).
- [ ] **Step 5:** Commit (com aprovação): `feat(app): streaming csv export`. Abrir o PR da M3.

---

## Milestone M4: UI (branch `feat/m4-ui`)

Regra para todas as tasks da M4: a lógica fica em `state/` (testada sem GUI); `ui/` só desenha a partir do estado e devolve intenções (`enum Intent`). Cada task adiciona as suas variantes de `Intent`, que `app.rs` traduz em `Command`.

### Task 25: Shell do app, tema e Home

**Files:** Create: `crates/crustdb-app/src/app.rs`, `ui/mod.rs`, `ui/theme.rs`, `ui/home.rs`, `state/mod.rs`, `state/workspace.rs`; Modify: `main.rs`

**Interfaces:**
- Produces: `struct CrustApp` (`impl eframe::App`) e `struct AppState { config: LoadedConfig, workspaces: Vec<WorkspaceState>, active: Option<WorkspaceId> }` (a Task 33 acrescenta `dialogs: DialogStack`); `fn AppState::apply(&mut self, e: Event)`; `enum Intent` (cada task adiciona as suas variantes); `fn install_theme(ctx: &egui::Context)` (dark + fonte `egui-phosphor`).

- [ ] **Step 1: Testes que falham:** `apply_connected_marks_workspace_online`; `apply_connect_failed_keeps_form_open_with_error`; `delete_connection_requires_confirmation`. Smoke `egui_kittest`: `home_lists_saved_connections`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::workspace ui::home`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. O `update()` drena o `Receiver<Event>` sem bloquear (`try_recv`). Logging com `tracing-subscriber` em stderr + arquivo de log, nível via `CRUSTDB_LOG`. A posição da janela usa a persistência do eframe.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): app shell, dark theme and home screen`.

### Task 26: Formulário de conexão

**Files:** Create: `crates/crustdb-app/src/state/connection_form.rs`, `ui/connection_form.rs`

**Interfaces:**
- Consumes: `validate` (Task 17), `parse_connection_url` (Task 8), `SecretStore` (Task 18).
- Produces: `struct ConnectionForm { draft: ConnectionConfig, password: SecretString, save_in_keyring: bool, ssh_password: SecretString, ssh_passphrase: SecretString, errors: Vec<FieldError>, warnings: Vec<String> }` com `fn import_url(&mut self, s: &str) -> Result<(), UrlError>`, `fn tls_downgraded(&self) -> bool`, `fn to_save(&self) -> Result<(ConnectionConfig, Vec<(SecretKind, SecretString)>), Vec<FieldError>>`.

- [ ] **Step 1: Testes que falham:** `default_ports_follow_driver`; `import_url_fills_fields_and_keeps_password_out_of_config`; `tls_below_verify_full_shows_warning`; `save_without_keyring_checkbox_stores_no_secret`; `keyring_unavailable_disables_checkbox_with_hint`; `test_button_sends_connect_with_ephemeral_workspace`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::connection_form`. Esperado: FAIL.
- [ ] **Step 3:** Implementar os campos e botões da spec §4.1.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): connection form with url import and validation`.

### Task 27: Sidebar

**Files:** Create: `crates/crustdb-app/src/state/tree.rs`, `ui/sidebar.rs`

**Interfaces:** Produces: `struct ObjectTree` com `fn expand(&mut self, node: NodePath) -> Option<Command>` (comando só se o nó ainda não estiver carregado), `fn apply_children(&mut self, req: RequestId, items: Vec<ObjectRef>)`, `fn filter(&mut self, q: &str)`, `fn visible(&self) -> impl Iterator<Item = VisibleNode>` e `fn refresh(&mut self, node: NodePath) -> Command`; `enum NodePath { Database(String), Schema(String, String), Folder { database: String, schema: Option<String>, kind: ObjectKind } }`.

- [ ] **Step 1: Testes que falham:** `pg_tree_has_schema_level_mysql_does_not`; `folders_match_spec_per_driver` (listas da spec §4.2); `expand_loads_once_and_caches`; `filter_matches_loaded_nodes_case_insensitive`; `double_click_table_opens_data_tab_function_opens_ddl`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::tree`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. O seletor de workspace fica no cabeçalho da sidebar (wireframe §4.11). F5 refaz o nó selecionado.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): lazy object tree sidebar`.

### Task 28: Aba de tabela: grid, paginação e sort

**Files:** Create: `crates/crustdb-app/src/state/table_tab.rs`, `ui/grid.rs`

**Interfaces:**
- Consumes: `display_cell` (Task 2), `TableQuery` (Task 6).
- Produces: `struct TableTab { object: ObjectRef, columns: Vec<ColumnMeta>, rows: Vec<Row>, page: u64, page_size: u32, has_more: bool, estimate: Option<u64>, sorts: Vec<Sort>, filters: Vec<Filter>, loading: Option<RequestId>, selected_row: Option<usize>, view: TableView }`; `enum TableView { Data, Structure, Ddl }`; `fn query(&self) -> TableQuery` (`limit = page_size + 1`, `offset = page * page_size`); `fn click_header(&mut self, column: &str, shift: bool)`; `fn next_page(&mut self)`; `fn prev_page(&mut self)`; `fn footer_label(&self) -> String`; `fn grid(ui: &mut egui::Ui, columns: &[ColumnMeta], rows: &[Row], selected: &mut Option<usize>)` (`egui_extras::TableBuilder` virtualizado, mesmo widget do resultado da Task 31).

- [ ] **Step 1: Testes que falham:**
```rust
#[test] fn pagination_next_enabled_only_when_limit_plus_one() { let mut t = tab(300); t.apply_page(rows(301)); assert!(t.has_more); assert_eq!(t.rows.len(), 300); t.apply_page(rows(120)); assert!(!t.has_more); }
#[test] fn sort_header_cycle_and_shift_multi() { let mut t = tab(300); t.click_header("a", false); assert_eq!(t.sorts, [asc("a")]); t.click_header("a", false); assert_eq!(t.sorts, [desc("a")]); t.click_header("b", true); assert_eq!(t.sorts, [desc("a"), asc("b")]); t.click_header("a", false); assert!(t.sorts.is_empty()); }
#[test] fn footer_shows_estimate_without_filters() { let t = tab_with(300, 1, Some(1_200_000), vec![]); assert_eq!(t.footer_label(), "Rows 301–600 of ~1.2M"); }
#[test] fn footer_hides_total_with_filters() { let t = tab_with(300, 0, Some(1_200_000), vec![filter_eq("a", "1")]); assert_eq!(t.footer_label(), "Rows 1–300"); }
#[test] fn sort_or_filter_change_resets_to_first_page() { /* page = 3, click_header → page == 0 */ }
```
- [ ] **Step 2:** `cargo test -p crustdb-app state::table_tab`. Esperado: FAIL.
- [ ] **Step 3:** Implementar o estado e o widget. `NULL` em cinza itálico. Aviso "unordered" quando não há PK nem sort. Botão Stop no rodapé enquanto `loading` está ativo (`CancelTab`). Ctrl+] e Ctrl+[ via Task 34.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): table data grid with pagination and multi-sort`.

### Task 29: Filtros, Structure, DDL e Row detail

**Files:** Create: `crates/crustdb-app/src/ui/filter_bar.rs`, `ui/structure.rs`, `ui/row_detail.rs`; Modify: `state/table_tab.rs`

**Interfaces:** Produces: `struct FilterDraft { column: String, op: Operator, input: String }` com `fn to_filter(&self, columns: &[ColumnMeta]) -> Result<Filter, String>` (`IN` via `parse_in_list`, `BETWEEN` com dois valores separados por vírgula); `fn row_detail_lines(columns: &[ColumnMeta], row: &Row) -> Vec<DetailLine>`, com `struct DetailLine { column: String, type_name: String, value: String }` (JSON formatado, `hex_dump` de 4 KiB, aviso "truncated at 1 MiB").

- [ ] **Step 1: Testes que falham:** `operator_list_follows_column_category`; `between_requires_two_values`; `apply_sends_fetch_with_and_combined_filters`; `server_filter_error_shown_in_bar`; `row_detail_pretty_prints_json_and_hex_dumps_binary`; `row_detail_marks_truncated`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::table_tab ui::row_detail`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. Ctrl+F abre a barra; Space alterna o row detail com foco no grid; os labels do row detail não são selecionáveis (D17). A DDL é realçada com `highlight` (Task 9).
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): filters, structure, ddl and row detail`.

### Task 30: Editor de query: texto, realce e execução

**Files:** Create: `crates/crustdb-app/src/state/query_tab.rs`, `ui/editor.rs`

**Interfaces:**
- Consumes: `split`, `statement_at`, `statements_in`, `highlight` (Tasks 3 e 9).
- Produces: `struct QueryTab { text: String, file: Option<PathBuf>, dirty: bool, database: Option<String>, tx: TxState, running: Option<RequestId>, error_marker: Option<(usize, Range<usize>)> }`; `fn targets(&self, cursor_byte: usize, selection: Option<Range<usize>>, all: bool) -> Vec<String>`; `fn char_to_byte(text: &str, char_idx: usize) -> usize`; `fn toggle_comment(text: &str, lines: Range<usize>) -> String`; `fn error_line(text: &str, position: usize) -> usize`.

- [ ] **Step 1: Testes que falham:** `run_current_uses_statement_under_cursor`; `run_with_selection_runs_statements_inside_selection`; `run_all_returns_every_statement`; `char_to_byte_offset_roundtrip` (texto com `ação🦀`); `toggle_comment_adds_and_removes_dash_dash`; `error_position_maps_to_line`; `highlight_disabled_above_1mib`; `current_statement_range_for_background`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::query_tab`. Esperado: FAIL.
- [ ] **Step 3:** Implementar `ui/editor.rs` com `egui::TextEdit::multiline` + `layouter` memoizado (cache por hash do texto, `egui::util::cache`), gutter de números de linha com marcador de erro, e fundo no statement atual (D40). Ctrl+Enter / Ctrl+Shift+Enter enviam `Evaluate` (o fluxo de diálogo é da Task 33).
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): sql editor with highlighting and run modes`.

### Task 31: Resultado, Messages, Load more, Stop e toolbar de transação

**Files:** Create: `crates/crustdb-app/src/ui/results.rs`; Modify: `state/query_tab.rs`

**Interfaces:** Produces: `struct ResultPane { header: Option<ResultHeader>, rows: Vec<Row>, exhausted: bool, messages: Vec<MessageLine>, tab: ResultTab }`; `enum ResultTab { Result, Messages }`; `struct MessageLine { index: usize, ok: bool, text: String, duration: Duration }`; `fn apply(&mut self, e: &Event)`. O toolbar emite `Intent::{Run, RunAll, Stop, Begin, Commit, Rollback, UseDatabase, LoadMore, ExportResult}`.

- [ ] **Step 1: Testes que falham:** `grid_shows_last_statement_with_rows` (D19); `messages_list_every_statement_with_affected_rows`; `load_more_hidden_when_exhausted`; `run_disabled_while_running_stop_enabled`; `db_selector_disabled_with_open_tx_on_pg`; `tx_badge_state_follows_event`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::query_tab ui::results`. Esperado: FAIL.
- [ ] **Step 3:** Implementar, reusando `ui::grid::grid` (Task 28).
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): result pane, messages and transaction toolbar`.

### Task 32: Autocomplete, arquivos e histórico

**Files:** Create: `crates/crustdb-app/src/ui/autocomplete.rs`, `ui/history_panel.rs`, `state/autocomplete.rs`; Modify: `state/query_tab.rs`

**Interfaces:**
- Consumes: `completion_context`, `rank`, `keywords` (Task 9); cache do `ObjectTree` (Task 27); `HistoryStore` (Task 18).
- Produces: `struct Autocomplete { open: bool, items: Vec<String>, selected: usize, replace: Range<usize> }` com `fn refresh(&mut self, ctx: &CompletionContext, tree: &ObjectTree, columns: &ColumnCache)` e `fn on_key(&mut self, key: egui::Key) -> AutocompleteAction { None, Accept(String, Range<usize>), Close }`; `struct ColumnCache` (colunas por `ObjectRef`, carregadas via `LoadStructure` de uma sessão Metadata); operações de arquivo `open_file`, `save`, `save_as` via `rfd`.

- [ ] **Step 1: Testes que falham:** `popup_opens_after_one_identifier_char`; `arrow_enter_tab_esc_behaviour`; `table_suggestions_after_from`; `column_suggestions_for_alias`; `max_50_items`; `save_clears_dirty_and_sets_title`; `close_dirty_prompts_save_discard_cancel`; `history_double_click_inserts_at_cursor`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::autocomplete state::query_tab`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. Com o popup aberto, as teclas ↑, ↓, Enter, Tab e Esc são consumidas antes de desenhar o `TextEdit` (`ctx.input_mut(|i| i.consume_key(...))`). A posição do popup vem da `galley` e do cursor de `TextEditOutput`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): autocomplete, sql files and history panel`.

### Task 33: Diálogos (segurança, senha, transação, host key)

**Files:** Create: `crates/crustdb-app/src/state/dialogs.rs`, `ui/confirm_dialog.rs`

**Interfaces:**
- Consumes: `Evaluation`, `Flag`, `Verdict` (Task 5).
- Produces: `enum Dialog { Safety { req: RequestId, evaluation: Evaluation, input: String, attempts: u8 }, CloseWithTx { tab: TabId }, DirtyFile { tab: TabId }, HostKey { ws: WorkspaceId, host: String, fingerprint: String }, DeleteConnection { id: ConnectionId } }`; `struct DialogStack`; `fn verify_password(input: &str, expected: Option<&SecretString>, database: &str) -> bool` (comparação em tempo constante; sem senha, compara com o nome do database, D25).

- [ ] **Step 1: Testes que falham:** `run_verdict_executes_without_dialog`; `confirm_dialog_lists_flagged_statements_with_reason_and_estimate`; `password_flow_three_failures_aborts`; `password_without_db_password_requires_database_name`; `constant_time_compare_rejects_prefix`; `cancel_sends_nothing`; `host_key_dialog_sends_decision`.
- [ ] **Step 2:** `cargo test -p crustdb-app state::dialogs`. Esperado: FAIL.
- [ ] **Step 3:** Implementar. `Event::Evaluated` com `Verdict::Run` envia `Execute` direto; com `Confirm` ou `Password`, empilha `Dialog::Safety`, e só a aprovação envia `Execute { evaluation: req }`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): safety, transaction and host key dialogs`.

### Task 34: Status bar e atalhos

**Files:** Create: `crates/crustdb-app/src/ui/status_bar.rs`, `src/keymap.rs`

**Interfaces:**
- Consumes: `ProcSampler` (Task 20); eventos `Ping`, `StatementDone`, `TxChanged`, `ExportProgress`.
- Produces: `struct StatusModel { connection: Option<(String, ServerVersion, bool)>, tls: Option<TlsMode>, ssh: bool, tx: TxState, threshold: u64, last_query: Option<QueryTiming>, rows: Option<u64>, latency: Option<Duration>, proc: Option<ProcStats>, exports: Vec<(RequestId, u64)> }`; `struct QueryTiming { first_row: Duration, total: Duration, render: Duration }`; `enum Focus { Editor, Grid, Other }`; `fn keymap(input: &egui::InputState, focus: Focus) -> Option<Action>`, onde `Action` cobre todas as linhas da tabela §4.9 da spec.

- [ ] **Step 1: Testes que falham:** `status_icons_follow_tls_mode` (fechado/âmbar/cortado); `tooltip_contains_safety_rules`; `samples_once_per_second`; `every_spec_shortcut_maps_to_action` (tabela §4.9 inteira, uma asserção por linha); `space_toggles_row_detail_only_with_grid_focus`.
- [ ] **Step 2:** `cargo test -p crustdb-app ui::status_bar keymap`. Esperado: FAIL.
- [ ] **Step 3:** Implementar com os ícones `egui-phosphor` e `ctx.request_repaint_after(Duration::from_secs(1))`.
- [ ] **Step 4:** Mesmo comando. Esperado: PASS. Smoke `egui_kittest`: `connect_and_open_table_renders_grid` e `run_query_shows_result_and_messages` (com `FakeDriver`) em `crates/crustdb-app/tests/ui_smoke.rs`.
- [ ] **Step 5:** Commit (com aprovação): `feat(ui): status bar indicators and keyboard shortcuts`. Abrir o PR da M4.

---

## Milestone M5: Release (branch `feat/m5-release`)

### Task 35: Cobertura, aceite e README

**Files:** Modify: `.github/workflows/ci.yml`, `README.md`; Create: `docs/acceptance/v1-checklist.md`

- [ ] **Step 1:** Ligar o job `coverage`: `cargo llvm-cov --workspace --lcov` com piso: `crustdb-core` ≥ 90%, drivers ≥ 70% (com integração), `crustdb-app/src/state` ≥ 70%; `ui/` excluído (`--ignore-filename-regex 'crustdb-app/src/ui/'`). Esperado: o job falha abaixo do piso.
- [ ] **Step 2:** Rodar todos os gates da spec §9. Esperado: tudo verde.
- [ ] **Step 3:** Preencher `docs/acceptance/v1-checklist.md` com AC-01…AC-16: para cada uma, o nome do teste automatizado ou o passo manual mais a evidência (log do servidor mostrando ausência de `COUNT(*)` em AC-06, contagem de execuções em AC-14, grep sem segredos em `$XDG_*` em AC-02).
- [ ] **Step 4:** README: build, run, matriz de versões efetivamente suportada (D6), limitações conhecidas da spec §7, caminhos de arquivos.
- [ ] **Step 5:** Commit (com aprovação): `docs: acceptance checklist, coverage gate and readme`.

### Task 36: Orçamentos de performance

**Files:** Create: `crates/crustdb-app/benches/ui_frame.rs`, `crates/crustdb-app/tests/perf.rs`; Modify: `README.md`

- [ ] **Step 1:** Testes `#[ignore = "perf"]`, rodados em release (`cargo test --release -p crustdb-app --test perf -- --include-ignored`):
  - `startup_first_frame_under_300ms`: env `CRUSTDB_TRACE_STARTUP=1` loga o `Instant` do primeiro frame.
  - `decode_and_render_1000x20_under_20ms`.
  - `scroll_100k_rows_frame_under_16ms`.
  - `rss_100k_rows_under_200mb`.
  - `export_100k_rows_per_second`.
  - `typing_5000_lines_frame_under_16ms`.
  - `wide_table_frame_time`: 250 colunas; registrar o valor, sem meta (spec §7).
  - Frames medidos com `egui_kittest` em modo headless.
- [ ] **Step 2:** Rodar. Registrar cada número no README (tabela métrica × meta × medido × commit). Para cada meta que falhar: profiling (`cargo flamegraph` ou `perf`) antes e depois da otimização, com evidência no PR. Se `typing_5000_lines` falhar após a otimização, aplicar D40 (desligar o realce acima de N linhas, N medido).
- [ ] **Step 3:** CPU ociosa < 1%: medir com `pidstat -p <pid> 1 30`, com uma conexão aberta, e registrar.
- [ ] **Step 4:** Commit (com aprovação): `perf: budgets measured and recorded`.

### Task 37: Empacotamento

**Files:** Create: `packaging/crustdb.desktop`, `packaging/crustdb.png`, `packaging/build-linux.sh`; Modify: `.github/workflows/ci.yml`

- [ ] **Step 1:** `build-linux.sh`: `cargo build --release`, `linuxdeploy` (ou equivalente) com `.desktop` e ícone, e geração de `crustdb-<versão>-x86_64.AppImage` e `crustdb-<versão>-x86_64-linux.tar.gz` (binário + `.desktop` + ícone).
- [ ] **Step 2:** Verificar num container limpo (`ubuntu:24.04` com sessão X virtual `xvfb`): o AppImage abre e o primeiro frame é logado. Esperado: exit 0.
- [ ] **Step 3:** Job `release` no CI, só com tag `v*`. Ele publica os artefatos e não roda em push.
- [ ] **Step 4:** Commit (com aprovação): `build: appimage and tarball packaging`. Abrir o PR da M5. Revisão final do branch inteiro com o agente `diff-reviewer` (Opus) antes do merge.
