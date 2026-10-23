# Changelog

Histórico de entregas concluídas via workflow autônomo.

---

## 2026-10-22 — #58
Testes unitários para o `tools/icons_generator`: a geração foi extraída de `main` para `generate(input, out_root)`, cobrindo entrada ausente, PNGs Linux, `icon.ico` Windows e iconset macOS.

## 2026-10-22 — #34
Rodapé discreto fixado no fim da tela inicial/conexão com a capa e um link para o livro *Desbravando Rust* (abre o site no navegador do sistema).

## 2026-10-22 — #33
Spec de melhorias de UI/UX do crustdb-app (`docs/specs/2026-10-22-crustdb-uiux-spec.md`): 46 melhorias (P1 15, P2 21, P3 10) com problema, evidência, proposta, prioridade e aceite por tela; itens conflitantes com a spec v1 movidos para backlog v2. Apenas documentação.

## 2026-10-22 — #35
Tela de conexão: duplo clique conecta na conexão salva, ícone colorido por engine (PostgreSQL/MySQL) e conexão online mostra "Active" com botão "Open" que reusa o workspace vivo em vez de abrir outro.

## 2026-10-22 — #32
Logo do cRustDB (`assets/imgs/logo2.png`) no topo da tela de conexão, via `include_image!` com altura máxima de 96px.

## 2026-10-22 — #3
Remove `libgtk-3-dev` dos steps apt-get do `ci.yml` (clippy, test, coverage, integration, release) e do README: `Cargo.lock` não tem crates gtk/gdk/glib/ashpd, `rfd` usa backend xdg-portal e fontconfig é carregado via dlopen, então `libfontconfig-dev` também não é necessário no build.

## 2026-10-22 — #45
fmt, clippy e deny rodam em paralelo; test depende dos três e usa `cargo nextest run --workspace --fail-fast`, parando no primeiro teste quebrado. Job coverage envia `lcov.info` ao Codecov (`codecov-action@v5`, secret `CODECOV_TOKEN`) e o README ganha badge de coverage.

## 2026-10-22 — #43
Avaliação técnica da troca de Apache-2.0 para BSL 1.1 e alternativas (FSL, ELv2, PolyForm, AGPL-3.0, Commons Clause, dual license), com recomendação e riscos em `docs/license-evaluation.md`. Licença trocada para BSL 1.1 (`BUSL-1.1`): Licensor Desbravando-Rust, uso de produção livre exceto revenda, cada versão vira Apache-2.0 após 4 anos. ADR em `docs/adr/0001-licenca-bsl-1.1.md`.

## 2026-10-22 — #41

Pipeline de CI roda uma vez por commit (push só em `main` e tags `v*`, mais `pull_request`), com concurrency que cancela runs antigos só em PR. Cache cargo em clippy, test, coverage e integration; cadeia fail-fast fmt → clippy/deny → test → coverage/integration → release. Matriz de integration sem a linha postgres:latest/mysql:8.4 (coberta pelo coverage). Badge `ci` no README.

## 2026-10-22 — #2

Documenta em `deny.toml` a cadeia de dependências e o gatilho de revisão de cada aviso RUSTSEC ignorado (ttf-parser; zvariant/zbus_names/zvariant_derive). Só comentários, sem mudança de entradas.

## 2026-10-22 — #37

Corrige falhas de pipeline: `deny` (feature `png` only no `image` do icons_generator, bumps de `rustls-webpki` e `webpki-roots`), skip de `keyring_roundtrip` em integration/coverage e testes `perf` retornam cedo em debug.

## 2026-10-22 — #36

Aplica `cargo fmt` em `tools/icons_generator/src/main.rs`, que fazia `cargo fmt --all --check` falhar em todas as branches. Mudanças só de formatação, sem mudança de comportamento.
