# Changelog

Histórico de entregas concluídas via workflow autônomo.

---

## 2026-10-22 — #2

Documenta em `deny.toml` a cadeia de dependências e o gatilho de revisão de cada aviso RUSTSEC ignorado (ttf-parser; zvariant/zbus_names/zvariant_derive). Só comentários, sem mudança de entradas.

## 2026-10-22 — #36

Aplica `cargo fmt` em `tools/icons_generator/src/main.rs`, que fazia `cargo fmt --all --check` falhar em todas as branches. Mudanças só de formatação, sem mudança de comportamento.
