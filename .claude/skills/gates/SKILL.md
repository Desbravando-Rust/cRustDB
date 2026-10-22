---
name: gates
description: Run crustdb's local quality gates (fmt, clippy -D warnings, tests, cargo deny) plus Docker integration tests for any driver crate touched in the diff. Use before asking to commit or when the user asks to verify changes.
---

Run from the repo (or worktree) root, in order. Stop at the first failure, show only the decisive error lines, fix or report.

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo deny check`
5. Driver integration (Docker): check `git diff --name-only HEAD` (plus untracked files). For each of `crates/crustdb-postgres/` and `crates/crustdb-mysql/` that appears, run `cargo test -p <crate> -- --include-ignored`. Also run both if `crates/crustdb-core/` or `crates/crustdb-testkit/` changed in a way the drivers consume (traits, dialect, testkit helpers). Skip otherwise and say so.

Report a terse table: gate, pass/fail, and for failures the first relevant error line. Do not commit.
