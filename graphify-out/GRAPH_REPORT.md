# Graph Report - crustdb  (2026-10-18)

## Corpus Check
- 57 files · ~107,072 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 851 nodes · 2044 edges · 25 communities
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 110 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `45858db7`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- DriverError
- start_mysql
- tests.rs
- ColumnMeta
- Session
- crustdb-core/src/dialect.rs
- Milestone M4: UI (branch `feat/m4-ui`)
- DriverKind
- driver.rs
- crustdb-postgres/tests/integration.rs
- crustdb-postgres/src/connect.rs
- crustdb-mysql/src/types.rs
- safety.rs
- 4. Requirements
- complete.rs
- split.rs
- PgSession
- crustdb-mysql/tests/introspect.rs
- crustdb-postgres/tests/introspect.rs
- format.rs
- README.md
- .ui
- CLAUDE.md

## God Nodes (most connected - your core abstractions)
1. `DriverError` - 70 edges
2. `Session` - 45 edges
3. `start_mysql()` - 35 edges
4. `ObjectRef` - 31 edges
5. `start_postgres()` - 30 edges
6. `ColumnMeta` - 29 edges
7. `classify()` - 26 edges
8. `DriverKind` - 25 edges
9. `MysqlSession` - 25 edges
10. `split()` - 22 edges

## Surprising Connections (you probably didn't know these)
- `insert_values_uses_tuple_count()` --calls--> `classify()`  [INFERRED]
  crates/crustdb-core/src/safety.rs → crates/crustdb-core/src/sql/classify/mod.rs
- `bench()` --calls--> `classify()`  [INFERRED]
  crates/crustdb-core/benches/sql.rs → crates/crustdb-core/src/sql/classify/mod.rs
- `bench()` --calls--> `highlight()`  [INFERRED]
  crates/crustdb-core/benches/sql.rs → crates/crustdb-core/src/sql/highlight.rs
- `bench()` --calls--> `split()`  [INFERRED]
  crates/crustdb-core/benches/sql.rs → crates/crustdb-core/src/sql/split.rs
- `page()` --calls--> `build_table_query()`  [INFERRED]
  crates/crustdb-mysql/tests/introspect.rs → crates/crustdb-core/src/dialect.rs

## Import Cycles
- 2-file cycle: `crates/crustdb-mysql/src/conn_task.rs -> crates/crustdb-mysql/src/session.rs -> crates/crustdb-mysql/src/conn_task.rs`

## Communities (25 total, 0 thin omitted)

### Community 0 - "DriverError"
Cohesion: 0.06
Nodes (58): Fetched, ResultHeader, Vec, DriverError, ErrorKind, Option, String, Active (+50 more)

### Community 1 - "start_mysql"
Cohesion: 0.08
Nodes (63): ContainerAsync, CopyTargetOptions, all(), cancel_long_query(), dropped_session_kills_paused_read(), editor(), explain_rows_for_update_delete(), failed_drain_is_reported_on_next_command() (+55 more)

### Community 2 - "tests.rs"
Cohesion: 0.07
Nodes (42): ExecMode, FlagReason, base_class(), Classified, classify(), DestructiveKind, DmlKind, explain_analyse() (+34 more)

### Community 3 - "ColumnMeta"
Cohesion: 0.07
Nodes (30): col(), col(), ColumnMeta, TypeCategory, col(), MysqlDialect, String, col() (+22 more)

### Community 4 - "Session"
Cohesion: 0.13
Nodes (37): Session, ColumnDetail, Completion, ConnectionId, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef (+29 more)

### Community 5 - "crustdb-core/src/dialect.rs"
Cohesion: 0.07
Nodes (34): build(), build_err(), build_table_query(), BuiltQuery, contains_escapes_wildcards(), default_sort_uses_pk(), empty_columns_selects_star(), escape_like() (+26 more)

### Community 6 - "Milestone M4: UI (branch `feat/m4-ui`)"
Cohesion: 0.04
Nodes (46): cRustDB v1 Implementation Plan, Global Constraints, Milestone M0: Foundation (branch `feat/m0-foundation`), Milestone M1: Core (branch `feat/m1-core`), Milestone M2: Drivers (branch `feat/m2-drivers`), Milestone M3: App infra (branch `feat/m3-app-infra`), Milestone M4: UI (branch `feat/m4-ui`), Milestone M5: Release (branch `feat/m5-release`) (+38 more)

### Community 7 - "DriverKind"
Cohesion: 0.08
Nodes (28): Dialect, Send, Sync, CancelHandle, Driver, Introspector, Send, Sync (+20 more)

### Community 8 - "driver.rs"
Cohesion: 0.08
Nodes (34): ConnectTarget, Param, Duration, Option, PathBuf, Result, SecretString, String (+26 more)

### Community 9 - "crustdb-postgres/tests/integration.rs"
Cohesion: 0.13
Nodes (35): all(), auth_failure_maps_to_auth_kind(), cancel_handle_cancels_running_query(), cancel_long_query(), connects_with_each_tls_mode(), cursor_inside_user_transaction_keeps_it_open(), disable_against_tls_required_server_fails_cleanly(), editor() (+27 more)

### Community 10 - "crustdb-postgres/src/connect.rs"
Cohesion: 0.12
Nodes (30): CertificateDer, connect(), err(), Lenient, map_err(), message_with_cause(), old_tls_maps_to_d39_message(), roots() (+22 more)

### Community 11 - "crustdb-mysql/src/types.rs"
Cohesion: 0.11
Nodes (25): Cell, Box, LineIndex, Vec, header(), Column, binary_protocol_dates_and_times(), category_for_column() (+17 more)

### Community 12 - "safety.rs"
Cohesion: 0.12
Nodes (14): Assessment, batch_takes_strictest_verdict(), estimate_need(), EstimateNeed, eval(), eval_batch(), Evaluation, Flag (+6 more)

### Community 13 - "4. Requirements"
Cohesion: 0.07
Nodes (28): 10. Out of scope v1 (backlog v2), 11. Left to the builder, 1. Technical overview, 2. Architecture impact, 3. Decisions and assumptions, 4.10 Export CSV, 4.11 Layout (wireframe), 4.1 Conexões (+20 more)

### Community 14 - "complete.rs"
Cohesion: 0.16
Nodes (23): after_alias_dot_expects_column_with_qualifier(), after_from_expects_table(), completion_context(), CompletionContext, ctx(), cursor_after_semicolon_starts_new_statement(), Expect, expect_at() (+15 more)

### Community 15 - "split.rs"
Cohesion: 0.18
Nodes (25): delimiter_line_ignores_trailing_comment(), handles_mysql_delimiter(), handles_pg_dollar_quotes(), ignores_semicolons_in_strings_and_comments(), long_single_line_many_statements(), mysql_multibyte_near_start_does_not_panic(), mysql_national_string_with_semicolon(), pg_escaped_string_with_semicolon() (+17 more)

### Community 16 - "PgSession"
Cohesion: 0.13
Nodes (16): Active, CancelToken, Client, bench(), Product, ServerVersion, parse_version(), PgCancel (+8 more)

### Community 17 - "crustdb-mysql/tests/introspect.rs"
Cohesion: 0.24
Nodes (23): app(), connect(), contains_is_case_insensitive_under_binary_collation(), ddl_for_each_object_kind(), f(), fetches_page_with_filters_and_sorts(), filter_value_injection_is_inert(), grid_table() (+15 more)

### Community 18 - "crustdb-postgres/tests/introspect.rs"
Cohesion: 0.29
Nodes (19): connect(), ddl_for_each_object_kind(), fetches_page_with_filters_and_sorts(), filter_value_injection_is_inert(), grid_table(), json_column_can_be_sorted(), lists_tree_levels_per_driver(), names() (+11 more)

### Community 19 - "format.rs"
Cohesion: 0.14
Nodes (8): Cow, display_cell(), hex_dump(), hex_preview(), human_size(), pretty_json(), Option, String

### Community 20 - "README.md"
Cohesion: 0.12
Nodes (15): Not ready to commit?, Pick your new home, So you're on Windows, Still here?, What you get instead, What you'll be leaving behind, Building, Development (+7 more)

### Community 21 - ".ui"
Cohesion: 0.29
Nodes (5): App, main(), Result, Frame, Ui

### Community 22 - "CLAUDE.md"
Cohesion: 0.33
Nodes (4): Code rules, Don't read, Gates, Git

## Knowledge Gaps
- **81 isolated node(s):** `Gates`, `Code rules`, `Git`, `Don't read`, `What it is` (+76 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `DriverKind` connect `DriverKind` to `tests.rs`, `ColumnMeta`, `Session`, `crustdb-core/src/dialect.rs`, `driver.rs`, `complete.rs`, `split.rs`?**
  _High betweenness centrality (0.200) - this node is a cross-community bridge._
- **Why does `DriverError` connect `DriverError` to `ColumnMeta`, `Session`, `DriverKind`, `driver.rs`, `crustdb-postgres/src/connect.rs`, `PgSession`?**
  _High betweenness centrality (0.109) - this node is a cross-community bridge._
- **Why does `classify()` connect `tests.rs` to `PgSession`, `DriverError`, `safety.rs`, `DriverKind`?**
  _High betweenness centrality (0.096) - this node is a cross-community bridge._
- **Are the 26 inferred relationships involving `start_mysql()` (e.g. with `cancel_long_query()` and `dropped_session_kills_paused_read()`) actually correct?**
  _`start_mysql()` has 26 INFERRED edges - model-reasoned connections that need verification._
- **Are the 25 inferred relationships involving `start_postgres()` (e.g. with `auth_failure_maps_to_auth_kind()` and `cancel_handle_cancels_running_query()`) actually correct?**
  _`start_postgres()` has 25 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Gates`, `Code rules`, `Git` to the rest of the system?**
  _81 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `DriverError` be split into smaller, more focused modules?**
  _Cohesion score 0.057703081232493 - nodes in this community are weakly interconnected._