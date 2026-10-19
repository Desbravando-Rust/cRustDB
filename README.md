<p align="center">
  <img src="imgs/logo-transparent.png" alt="CrustDB" width="240">
</p>

<p align="center">
  Native Linux desktop client for PostgreSQL and MySQL/MariaDB, written in Rust with egui.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-pre--alpha-orange" alt="Status: pre-alpha">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0">
  <img src="https://img.shields.io/badge/rust-1.99.1-b7410e?logo=rust" alt="Rust 1.99.1">
  <img src="https://img.shields.io/badge/edition-2024-b7410e" alt="Edition 2024">
  <img src="https://img.shields.io/badge/unsafe-forbidden-success" alt="unsafe forbidden">
  <br>
  <img src="https://img.shields.io/badge/platform-Linux%20x86__64-informational?logo=linux&logoColor=white" alt="Platform: Linux x86_64">
  <img src="https://img.shields.io/badge/display-X11%20%7C%20Wayland-informational" alt="X11 and Wayland">
  <img src="https://img.shields.io/badge/PostgreSQL-%E2%89%A5%2012-336791?logo=postgresql&logoColor=white" alt="PostgreSQL 12+">
  <img src="https://img.shields.io/badge/MySQL-%E2%89%A5%205.6.5-4479a1?logo=mysql&logoColor=white" alt="MySQL 5.6.5+">
  <img src="https://img.shields.io/badge/MariaDB-%E2%89%A5%2010.0.5-003545?logo=mariadb&logoColor=white" alt="MariaDB 10.0.5+">
</p>

> **Pre-release.** All v1 features are implemented; packaging and performance budgets are being finalized. Acceptance status per criterion: [`docs/acceptance/v1-checklist.md`](docs/acceptance/v1-checklist.md).

## What it is

CrustDB browses data in a **read-only grid** and runs **raw SQL** with guards against accidental writes. It is fast, small and keeps your secrets off disk. No data editing, diagrams, import, backup or AI: v1 does a few things and does them well.

## Features

- **Connections**: saved connections, import from `postgres://`, `postgresql://`, `mysql://`, `mariadb://` URLs, several connections open at once in one window.
- **TLS** via `rustls` (TLS 1.2+ only): `disable`, `require`, `verify-ca`, `verify-full` (default). No silent `prefer` downgrade.
- **SSH tunnel** via `russh`: password, key file (+ passphrase) or agent, strict `known_hosts`.
- **Secrets** in the system keyring (Secret Service). Without one, CrustDB asks every time and never writes passwords to disk.
- **Object tree**: databases, schemas, tables, views, materialized views, functions, procedures, triggers, sequences and types, loaded lazily.
- **Table tab**: virtualized grid, `LIMIT/OFFSET` pagination, multi-column sort, typed filters (always bound as parameters), structure (columns, indexes, foreign keys), DDL and a row detail panel. Row counts are estimates only, never `COUNT(*)`.
- **SQL editor**: highlighting, autocomplete, run statement under cursor / selection / all, cancel, "Load more" from a server-side cursor, history, open/save `.sql` files.
- **Transactions**: autocommit by default, Begin / Commit / Rollback per tab, state shown in the status bar.
- **Safety policy**: destructive or multi-row statements ask for confirmation; DML estimated (via `EXPLAIN`) above a per-connection threshold asks for the password.
- **CSV export** (RFC 4180), streamed with constant memory and cancellable.
- **Status bar**: TLS/SSH state, transaction badge, query timings, ping latency, process CPU% and RSS.

## Supported databases

| Database | Minimum version |
|---|---|
| PostgreSQL | 12 |
| MySQL | 5.6.5 |
| MariaDB | 10.0.5 |

Integration suite results (full `cargo test --workspace -- --include-ignored`, 2026-10-18):

| PostgreSQL | MySQL / MariaDB | Result |
|---|---|---|
| 12 | MySQL 8.4 | pass |
| latest | MySQL 5.6 | pass |
| latest | MySQL 5.7 (5.7.44) | pass, **no TLS** (see limitations) |
| latest | MariaDB 10.0 | pass, **no TLS** (see limitations) |
| latest | MariaDB latest | pass |

The floors may move up if the driver fails integration tests on them; any change will be recorded here. SQLite is planned for v2 ([#26](https://github.com/Desbravando-Rust/crustdb/issues/26)).

## Building

> **On Windows?** [Read this first](docs/windows.md). We prepared you a soft landing.

Requirements: Linux x86_64, the Rust toolchain pinned in `rust-toolchain.toml` (installed automatically by `rustup`), and the GUI system libraries:

```sh
sudo apt-get install -y libxkbcommon-dev libwayland-dev libgtk-3-dev
```

Build and run:

```sh
cargo build --release
cargo run -p crustdb-app
```

## Workspace layout

```
crustdb-app ──► crustdb-postgres ──► crustdb-core
     │      └─► crustdb-mysql    ──► crustdb-core
     └──────────────────────────────► crustdb-core
```

| Crate | Role |
|---|---|
| `crustdb-core` | Domain types, driver traits, SQL split/classify/highlight/autocomplete, safety policy. No IO, no tokio, no egui. |
| `crustdb-postgres` | PostgreSQL driver (`tokio-postgres`). |
| `crustdb-mysql` | MySQL/MariaDB driver (`mysql_async`). |
| `crustdb-app` | egui UI, tokio runtime and actors, SSH tunnel, storage, metrics, CSV export. Binary: `crustdb`. |
| `crustdb-testkit` | Test support (containers and certificates). |

## Development

Quality gates (run in CI and before each commit):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo build --release
```

CI also enforces line-coverage floors (`cargo llvm-cov`, `ui/` excluded): `crustdb-core` ≥ 90%, drivers ≥ 70%, `crustdb-app/src/state` ≥ 70%.

Integration tests run against PostgreSQL, MySQL and MariaDB in Docker via `testcontainers`. The design and plan live in [`docs/specs/`](docs/specs/).

## Files on disk

| Path | Content |
|---|---|
| `$XDG_CONFIG_HOME/crustdb/connections.json` | Saved connections, no secrets (0600) |
| `$XDG_CONFIG_HOME/crustdb/known_hosts` | Accepted SSH host keys (OpenSSH format) |
| `$XDG_DATA_HOME/crustdb/history/<id>.jsonl` | Query history per connection, last 10,000 entries (0600) |

## Known limitations

- The local port of the SSH tunnel is reachable by other local users while the session is open.
- Query history stores SQL in plain text, including any secret typed into a query.
- CSV export does not neutralize spreadsheet formulas (CSV injection).
- Pagination uses `OFFSET`, so very deep pages are slow. Keyset pagination is planned for v2.
- MariaDB 10.0 and MySQL 5.7 (5.7.44 image, OpenSSL 1.0) cannot negotiate TLS with `rustls` because the server only offers DHE suites: use `disable` through an SSH tunnel or a trusted network.
- MySQL/MariaDB `verify-ca` behaves like `verify-full` (a certificate whose name does not match the host is rejected); use `require` if the certificate lacks the hostname.
- The MySQL driver trusts the embedded Mozilla roots (`webpki-roots`) instead of the system store; use `ca_path` for a private CA.
- SSH RSA keys (user and host) are supported through `russh`'s `rsa` feature, which pulls in the `rsa` crate (RUSTSEC-2023-0071, Marvin timing leak, no upstream patch). The advisory is ignored in `deny.toml`; it matters only to an observer collecting many signatures from one session. Prefer ed25519 or ECDSA keys when you can.
- A one-column CSV export writes a `NULL` cell as an empty line.
- The grid does not virtualize columns; tables with more than 200 columns may render slower.

## License

Licensed under [Apache-2.0](LICENSE).
