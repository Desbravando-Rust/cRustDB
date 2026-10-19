# v1 acceptance checklist

Spec §8 (AC-01…AC-16). Each row names the automated test that covers it, or the manual step and its evidence.
Tests marked `docker` are `#[ignore = "docker"]` and run with `cargo test -p <crate> -- --include-ignored`.

Status: **auto** = covered by a passing test · **manual** = needs a human run · **pending** = depends on T36 (perf).

| AC | Status | Evidence |
|---|---|---|
| AC-01 Save PG/MySQL connection, reopen, connect without retyping the password | manual | Unit: `crustdb-app` `storage::secrets::keyring_roundtrip`. Manual: save a connection with "store in keyring" on a machine with Secret Service, restart, connect. |
| AC-02 No Secret Service → password asked each time; no secret on disk | auto + manual | `state::connection_form::save_without_keyring_checkbox_stores_no_secret`, `storage::config::config_roundtrip_has_no_secrets`. Manual: after a session, `grep -r "<password>" "$XDG_CONFIG_HOME/crustdb" "$XDG_DATA_HOME/crustdb"` returns nothing. |
| AC-03 SSH (password, key, agent) with `verify-full` | auto (docker) | `crustdb-app/tests/ssh.rs`: `ssh_tunnel_with_key_password_agent`, `verify_full_via_ssh_checks_real_hostname`, `unknown_host_key_rejected`, `changed_host_key_blocks`. |
| AC-04 URL import fills the form | auto | `state::connection_form::import_url_fills_fields_and_keeps_password_out_of_config`; parser in `crustdb-core` `url` tests. |
| AC-05 Sidebar lists all levels and folders of §4.2 on both databases | auto (docker) | `state::tree::folders_match_spec_per_driver`; `lists_tree_levels_per_driver` in `crustdb-postgres/tests/introspect.rs` and `crustdb-mysql/tests/introspect.rs`. |
| AC-06 First page of a table opens without `COUNT(*)` | auto + evidence | `row_estimate_without_count` (both drivers, docker). Server log evidence: PostgreSQL with `log_statement=all` running the `introspect` + `export_it` suites logged page fetches (`SELECT … FROM "schema"."table" … LIMIT`) and **0** `count(` statements other than the fixture's `CREATE MATERIALIZED VIEW` DDL. |
| AC-07 `=` on integer and `contains` on text; quotes and `%` do not break or inject | auto | `crustdb-core` `dialect::contains_escapes_wildcards`, `dialect::params_numbered_across_filters`; docker: `filter_value_injection_is_inert`, `fetches_page_with_filters_and_sorts`, `table_with_quoted_mixed_case_name_opens`. |
| AC-08 Multi-sort stable across pages | auto | `dialect::multi_sort_order_preserved`, `dialect::default_sort_uses_pk`, `state::table_tab_tests::sort_header_cycle_and_shift_multi`; docker: `numeric_sorts_are_not_lexicographic`. |
| AC-09 Structure, DDL and row detail correct for every object kind | auto (docker) + manual | `ddl_for_each_object_kind`, `structure_lists_columns_indexes_fks` (both drivers); `keymap::space_toggles_row_detail_only_with_grid_focus`. Manual: check row detail on one row per type category. |
| AC-10 Ctrl+Enter runs the statement under the cursor; Ctrl+Shift+Enter runs all; grid shows the last statement with rows | auto | `state::query_tab_tests::run_current_uses_statement_under_cursor`, `run_all_returns_every_statement`, `state::workspace_tests::run_selection_is_in_chars_not_bytes`, `state::result_pane::grid_shows_last_statement_with_rows`, `actors::editor::tests::run_all_stops_at_first_error`. |
| AC-11 `DELETE` without `WHERE` confirms; `UPDATE` estimated >1,000 rows asks for the password; Cancel executes nothing | auto | `crustdb-core` `safety::no_where_requires_confirm_even_if_small`, `safety::multi_row_requires_confirm`; `state::dialogs_tests::flag_level_password_requires_password_even_if_batch_says_confirm`; `actors::editor::tests::confirm_flow_cancel_executes_nothing`. |
| AC-12 Cancel `pg_sleep(60)` / `SLEEP(60)` in ≤ 1 s | auto (docker) | `cancel_handle_cancels_running_query` and `cancel_long_query` in both driver crates; `actors::editor::tests::cancel_tab_cancels_running_statement`. |
| AC-13 Begin → DML → Rollback undoes; closing a tab with an open transaction asks | auto | `actors::editor::tests::tx_state_follows_buttons_and_statements`, `close_tab_with_open_tx_requires_resolution`; docker: `transaction_states_and_implicit_commit_mysql`, `cursor_inside_user_transaction_keeps_it_open`. |
| AC-14 CSV export of filtered table and of the full query result, without re-executing | auto + evidence | `actors::export::tests::export_respects_filters_and_sorts`, `export_result_drains_without_reexecuting`; docker: `crustdb-app/tests/export_it.rs`. Same `log_statement=all` run as AC-06 over `export_it`. |
| AC-15 Status bar shows every §4.8 indicator with tooltips | auto + manual | `ui::status_bar::status_icons_follow_tls_mode`, `tooltip_contains_safety_rules`. Manual: connect over TLS + SSH, hover each indicator. |
| AC-16 §7 performance budgets measured and within target | pending | T36 (`crustdb-app/tests/perf.rs`, numbers in README). |

## Coverage floors (spec §9)

Measured with `cargo llvm-cov --workspace --lcov -- --include-ignored`, `crustdb-app/src/ui/` excluded.
CI job `coverage` fails below the floors.

| Area | Floor | Measured |
|---|---|---|
| `crustdb-core` | 90% | 96.7% |
| `crustdb-postgres` | 70% | 92.5% |
| `crustdb-mysql` | 70% | 94.8% |
| `crustdb-app/src/state` | 70% | 94.3% |
