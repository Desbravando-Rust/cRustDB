<p align="center">
  <img src="assets/imgs/logo.svg" alt="CrustDB" width="360">
</p>

<p align="center">
  Native Linux desktop client for PostgreSQL and MySQL/MariaDB, written in Rust with egui.
</p>

<p align="center">
  <a href="https://github.com/Desbravando-Rust/cRustDB/actions/workflows/ci.yml"><img src="https://github.com/Desbravando-Rust/cRustDB/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://codecov.io/gh/Desbravando-Rust/cRustDB"><img src="https://codecov.io/gh/Desbravando-Rust/cRustDB/branch/main/graph/badge.svg" alt="coverage"></a>
  <img src="https://img.shields.io/badge/status-pre--alpha-orange" alt="Status: pre-alpha">
  <img src="https://img.shields.io/badge/license-BUSL--1.1-blue" alt="License: BUSL-1.1">
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

**cRustDB** browses data in a **read-only grid** and runs **raw SQL** with guards against accidental writes. It is fast, small and keeps your secrets off disk. No data editing, diagrams, import, backup or AI: v1 does a few things and does them well.

## Features

- **Connections**: saved connections, import from `postgres://`, `postgresql://`, `mysql://`, `mariadb://` URLs, several connections open at once in one window.
- **TLS** via `rustls` (TLS 1.2+ only): `disable`, `require`, `verify-ca`, `verify-full` (default). No silent `prefer` downgrade.
- **SSH tunnel** via `russh`: password, key file (+ passphrase) or agent, strict `known_hosts`.
- **Secrets** in the system keyring (Secret Service). Without one, **cRustDB** asks every time and never writes passwords to disk.
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
sudo apt-get install -y libxkbcommon-dev libwayland-dev
```

Build and run:

```sh
cargo build --release
cargo run -p crustdb-app
```

### Packages

`packaging/build-appimage.sh` builds `crustdb-<version>-x86_64.AppImage` and `crustdb-<version>-x86_64-linux.tar.gz` into `target/dist/`. Build on the oldest Linux you want to support (CI uses Ubuntu 24.04, glibc 2.39): the binary needs at least the glibc it was built with. Tag `v*` publishes both files as a GitHub release.

The packages rely on the system graphics stack (a Vulkan or OpenGL driver, `libxkbcommon`, and X11 or Wayland libraries); they are not bundled. Pushing a `v*` tag runs the `release` job.

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

## Performance

Release build, Linux x86_64. Reproduce: `cargo test --release -p crustdb-app --test perf -- --include-ignored --test-threads=1 --nocapture`.
Frame times come from `egui_kittest` (layout and tessellation on the CPU, no GPU rasterization), so they exclude the GPU.

| Metric | Target | Measured | Status |
|---|---|---|---|
| Start to first frame | ≤ 300 ms | 117 ms (median of 5, Xvfb) | met |
| Idle CPU, 1 connection open | < 1% | 0.0% on the home screen (real display, 1 frame/s); 3.3% under Xvfb software rendering; not measured with a connection open | partial |
| Decode and render 1,000 × 20 cells | ≤ 20 ms | 16.2 ms (8.0 build + format, 8.2 frame) | met |
| First rows visible after the server's first row | ≤ 50 ms | not measured | open |
| Frame scrolling 100,000 × 20 rows | ≤ 16 ms | 7.9 ms mean, 10.2 ms worst | met |
| RSS with 100,000 × 20 text cells | ≤ 200 MB | 172 MB | met |
| CSV export | ≥ 100,000 rows/s, ≤ +20 MB | 186,000 rows/s encoding 20 columns, +0 MB (client side only; not measured end to end against a server) | partial |
| Typing in a 5,000-line file | ≤ 16 ms | 53 ms (highlighting off); 13.7 ms at 700 lines with highlighting | **not met** |
| 250-column table | none (spec §7) | 102 ms mean | recorded |

Typing: highlighting turns off above 750 lines (measured limit). The 5,000-line target needs more than that, because egui's `TextEdit` lays out the whole text on every edit (a bare `TextEdit` with 5,000 lines already takes ~17 ms) and the SQL analysis passes scan the whole text per edit. Reaching it needs a virtualized editor, planned for v2; `perf.rs` guards the current ceiling (100 ms).

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

Licensed under the [Business Source License 1.1](LICENSE). Production use is allowed, including inside companies, as long as you do not sell cRustDB or a product or service built mainly on it. Each version converts to Apache-2.0 four years after its release. See [docs/license-evaluation.md](docs/license-evaluation.md).
