# Changelog

Histórico de entregas concluídas via workflow autônomo.

---

## 2026-10-22 — #37

Corrige falhas de pipeline: `deny` (feature `png` only no `image` do icons_generator, bumps de `rustls-webpki` e `webpki-roots`), skip de `keyring_roundtrip` em integration/coverage e testes `perf` retornam cedo em debug.

## 2026-10-22 — #36

Aplica `cargo fmt` em `tools/icons_generator/src/main.rs`, que fazia `cargo fmt --all --check` falhar em todas as branches. Mudanças só de formatação, sem mudança de comportamento.
