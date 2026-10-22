# Changelog

Histórico de entregas concluídas via workflow autônomo.

---

## 2026-10-22 — #41

Pipeline de CI roda uma vez por commit (push só em `main` e tags `v*`, mais `pull_request`), com concurrency que cancela runs antigos só em PR. Cache cargo em clippy, test, coverage e integration; cadeia fail-fast fmt → clippy/deny → test → coverage/integration → release. Matriz de integration sem a linha postgres:latest/mysql:8.4 (coberta pelo coverage). Badge `ci` no README.

## 2026-10-22 — #2

Documenta em `deny.toml` a cadeia de dependências e o gatilho de revisão de cada aviso RUSTSEC ignorado (ttf-parser; zvariant/zbus_names/zvariant_derive). Só comentários, sem mudança de entradas.

## 2026-10-22 — #37

Corrige falhas de pipeline: `deny` (feature `png` only no `image` do icons_generator, bumps de `rustls-webpki` e `webpki-roots`), skip de `keyring_roundtrip` em integration/coverage e testes `perf` retornam cedo em debug.

## 2026-10-22 — #36

Aplica `cargo fmt` em `tools/icons_generator/src/main.rs`, que fazia `cargo fmt --all --check` falhar em todas as branches. Mudanças só de formatação, sem mudança de comportamento.
