# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Spec and plan (Portuguese, `§n`/`Dn` refs): `docs/specs/2026-10-17-crustdb-spec.md` and `docs/specs/2026-10-17-crustdb-plan.md` (tasks 1–37, milestones M0–M5; read the task you are on, not the whole file).

## Gates

Run in this order; all must pass (`/gates` runs them):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

- Driver integration tests are `#[ignore = "docker"]` and start their own containers via testcontainers (no `DATABASE_URL`). Run them only for the driver crate you touched: `cargo test -p crustdb-postgres -- --include-ignored` (same for `crustdb-mysql`). Images: `CRUSTDB_PG_IMAGE` (default `postgres:latest`), `CRUSTDB_MYSQL_IMAGE` (default `mysql:8.4`; MariaDB works).
- Binary: `cargo run -p crustdb-app` (binary is `crustdb`). Benches: `cargo bench -p crustdb-core`.
- Toolchain pinned to 1.99.1, edition 2024. Linux x86_64 only.

## Code rules

- Workspace lints: `unsafe_code = "forbid"`, `clippy::unwrap_used`/`expect_used = "deny"`. Every crate sets `lints.workspace = true`. Tests opt out with `#[allow(clippy::unwrap_used, clippy::expect_used)]` on the test module (integration test files use a crate-level `#![allow(...)]`, plus `clippy::err_expect`).
- Dependency direction: `crustdb-app` → `crustdb-postgres`/`crustdb-mysql` → `crustdb-core`. `crustdb-core` has no IO, no tokio, no egui. `crustdb-testkit` is dev-dependency only.
- Errors: `thiserror` in core and drivers; `anyhow` only in `crustdb-app/src/main.rs`. Secrets as `secrecy::SecretString`.
- Generated SQL: user values are always bound parameters; identifiers always go through `Dialect::quote_ident`.
- Add dependencies with `cargo add` (never type versions from memory). License BUSL-1.1 (set once in `[workspace.package]`; crates use `license.workspace = true`). Files ≤ 500 lines.
- UI text is English.
- Tests follow AAA: `// Arrange`, `// Act`, `// Assert` sections separated by a blank line (`// Act & Assert` when inseparable; omit `// Arrange` without setup). See `CONTRIBUTING.md`.

## Git

- Conventional Commits, lowercase imperative, scopes: `core`, `postgres`, `mysql`, `drivers`, `app`, `ui`, `test`, `docs`, `chore`.
- One branch/PR per milestone (`feat/m0-foundation` … `feat/m5-release`). Per-task worktrees on `wip/tNN` under `../crustdb-wt/`, brought into the milestone branch with a normal merge.
- Never push to `main`. Commit/push only with explicit approval. No AI mentions or `Co-Authored-By` in commits/PRs.

## Don't read

`target/`, `benchmarks/` (~560 MB competitor installers), `ruvector.db`, `agentdb.rvf*`, `.swarm/`, `.claude-flow/`, `.superpowers/`.


## Tests
- Always write tests with AAA's pattern


## Open an Issue to Avoid Forgetting
- When you come across an outstanding issue—whether small or large—and the rounds of review and correction have already been exhausted, open an issue so it doesn't get forgotten.
- If you spot something out of scope for this implementation, log an issue as well