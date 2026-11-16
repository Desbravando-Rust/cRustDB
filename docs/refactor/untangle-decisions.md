# Untangle refactor: decision log (#283)

Question asked of every piece: should these change together? Each fix is listed above the
problems it solves. Additions chosen: comments, tests.

## Fixes applied

- **Driver facts live on `DriverKind`** (`is_file`, `has_schemas`, `session_per_database`,
  `error_aborts_tx`, `ddl_commits_tx`, `ALL`). The app asks for a fact instead of comparing kinds.
  - PG session-per-database compared in `actors/workspace.rs`, `workspace/diagram.rs`,
    `editor/exec.rs` (`use_database`), `state/query_tab.rs` (`can_switch_db`).
  - PG schema level compared in `state/tree.rs` (x2), `state/tree_filter.rs`, `editor/mod.rs`.
  - "failed statement aborts the tx" written three times in the editor actor.
  - "DDL commits the tx" written as `== Mysql` in one place and `!pg` in another, so failed
    SQLite DDL reported an implicit commit (bug, regression test added).
  - SQLite-is-a-file compared in config validation, the connection form (state and UI),
    `Shared::target_for`, the file picker.
  - Driver combo listed the kinds twice; editor highlight/split cache keyed on a `mysql` bool
    that collapsed SQLite into PostgreSQL.
- **Shared driver plumbing in core**: `DriverError::{new, query, timeout, no_catalog_query}`,
  `NO_TLS12`, `driver::{drain_text, text_at, fetch_up_to}`, `ErrorKind::ends_session`,
  `store::DEFAULT_USER`.
  - `err()` copied in four drivers; the catalog row loop in three introspectors; `col()` x3;
    "no catalog query" x3; "connection timed out" x5; the TLS 1.2 message x3 (plus tests).
  - Bounded row reader copied in the data and editor actors.
  - Session-ending predicate repeated in the workspace, diagram and store actors.
- **Kind-to-dialect lookup sits beside the driver registry** (`runtime::dialect`), so adding a
  driver edits one file. Registry keyed by `DriverKind` (now `Hash`).
- **Typed events instead of sentinels and text matching**.
  - Diagram failures and store tree loads were `Notice { tab: TabId(0) }` that no tab shows:
    `Event::Diagram` now carries `Result`, store tree loads answer `ChildrenFailed`, workspace
    supervisors no longer emit `ActorCrashed { TabId(0) }` (their `ConnectFailed` covers it).
  - `state/query_tab.rs` recognised a refused/lost run by matching notice text and imported
    actor constants: `Event::RunStopped`.
- **One source per value**: `NOT_CONNECTED`/`not_connected()`, `theme::{SUCCESS, WARNING}`,
  highlight limits and pause timeout in their messages, `KEYRING_UNAVAILABLE`,
  `SecretKind::ALL` + `forget_secrets`, `keyspace::DEFAULT_NAMESPACE`, book cover/URL,
  `RequestId::{STATE_BASE, tree_base}` + one `internal_req`, `TableTab::read_only_reason`,
  `PENDING_NOTICE`, `ConnectionConfig::uses_client_cert`, `metadata_key`,
  `QueryTab::session_reset`.
- **Shortcuts come from the keymap** (`keymap::consume`, `keymap::hint`, `Action::Format`):
  the editor hardcoded its keys and toolbar hints spelled them by hand.
- **Surplus removed**: single-variant `enum Tab`, test-only `form_open` and `current_range`.
- **Comments**: removed ones restating code and ones the refactor made false.

## Kept on purpose

- Mailbox `select!` loops in data/editor/export actors: each has its own cancel policy.
- Data actor also treats `Timeout` as session-ending (a timed-out write may still run).
- TLS verifier/roots copies in pg and redis: tracked in #172 (core takes no rustls dependency).
- `only_home` MySQL case folding, MySQL `DELIMITER`, sidebar ER-diagram rule (#288).

## Not changed (with reason)

- `TableTab::new` defaults the driver; its one production caller sets it next line.
- New-label grey in `app/form.rs` vs `ui/labels.rs`, picker export extensions vs
  `Format::from_path`, the two token colour tables (editor vs grid use different palettes on
  purpose until a design decision), `runtime::no_actor` vs `Actors::dispatch` (#291).
- Doc comments (`///`) were not audited; only `//` comments were.
- Tests: about 700 of ~1,100 audited. `recreating_the_keyspace_bumps_the_epoch...` still
  asserts an internal id salt; rewriting it needs a UI harness and was left as is.

## Follow-up issues

#285 key_separator ignored, #286 config save side effect, #287 autocomplete dedup,
#288 SQLite ER diagram, #289 editor Protocol errors, #290 Redis db index rules,
#291 workspace commands without actor hang, #292 SSH error causes, #293 news sanitizing.
