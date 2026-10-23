# Changelog

Delivered work, grouped by date (newest first). One line per issue.

---

## 2026-10-23

- [#26](https://github.com/Desbravando-Rust/cRustDB/issues/26) — SQLite driver, part 2 of 2: wires `crustdb-sqlite` into the app (driver registration, file-only connection form, `sqlite://` URL import).
- [#26](https://github.com/Desbravando-Rust/cRustDB/issues/26) — SQLite driver, part 1 of 2: core support and new `crustdb-sqlite` crate (rusqlite). Not yet selectable in the UI; part 2 wires it into the app.

## 2026-10-22

- [#23](https://github.com/Desbravando-Rust/cRustDB/issues/23) — Editor result grid: click headers to sort (shift for multi-column) and a "Filter rows" box over loaded rows.
- [#61](https://github.com/Desbravando-Rust/cRustDB/issues/61) — Faster CI pipeline: coverage runs in parallel with test via nextest, deny without Docker, rust-cache saved only on main.
- [#54](https://github.com/Desbravando-Rust/cRustDB/issues/54) — Linux packages: `.deb`, `.rpm` and `.flatpak` alongside AppImage and tarball, built by `packaging/build-linux.sh` and published on `v*` tags. Windows/macOS moved to [#25](https://github.com/Desbravando-Rust/cRustDB/issues/25).
- [#58](https://github.com/Desbravando-Rust/cRustDB/issues/58) — Unit tests for `tools/icons_generator`; generation extracted into `generate(input, out_root)`.
- [#34](https://github.com/Desbravando-Rust/cRustDB/issues/34) — Home/connection screen footer promoting the *Desbravando Rust* book, opening the site in the system browser.
- [#33](https://github.com/Desbravando-Rust/cRustDB/issues/33) — UI/UX improvement spec for crustdb-app (`docs/specs/2026-10-22-crustdb-uiux-spec.md`), 46 items by priority. Docs only.
- [#35](https://github.com/Desbravando-Rust/cRustDB/issues/35) — Connection screen: double-click connects, engine-colored icons, and live connections show "Active" with an "Open" button that reuses the workspace.
- [#32](https://github.com/Desbravando-Rust/cRustDB/issues/32) — cRustDB logo at the top of the connection screen.
- [#3](https://github.com/Desbravando-Rust/cRustDB/issues/3) — Drop `libgtk-3-dev` from CI apt-get steps and the README; it is not needed to build.
- [#45](https://github.com/Desbravando-Rust/cRustDB/issues/45) — CI runs fmt, clippy and deny in parallel; tests use `cargo nextest` with fail-fast; coverage uploaded to Codecov with a README badge.
- [#43](https://github.com/Desbravando-Rust/cRustDB/issues/43) — License changed from Apache-2.0 to BSL 1.1 (`BUSL-1.1`), with evaluation in `docs/license-evaluation.md` and ADR 0001.
- [#41](https://github.com/Desbravando-Rust/cRustDB/issues/41) — CI runs once per commit with cargo caching, a fail-fast job chain and a trimmed integration matrix; `ci` badge in the README.
- [#2](https://github.com/Desbravando-Rust/cRustDB/issues/2) — Document the dependency chain and review trigger for each ignored RUSTSEC advisory in `deny.toml`. Comments only.
- [#37](https://github.com/Desbravando-Rust/cRustDB/issues/37) — Fix CI failures: `deny` (icons_generator `image` features, `rustls-webpki`/`webpki-roots` bumps), skip `keyring_roundtrip` in CI, and `perf` tests return early in debug.
- [#36](https://github.com/Desbravando-Rust/cRustDB/issues/36) — Apply `cargo fmt` to `tools/icons_generator/src/main.rs`. Formatting only.
