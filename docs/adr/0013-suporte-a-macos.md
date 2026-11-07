# Suporte a macOS (aarch64 e x86_64)

* **Status:** Aceito
* **Data:** 2026-11-07
* **Issue Relacionada:** #254 (fase 1 da #25)

## Contexto e Problema
O app é só Linux x86_64 (spec D1). A #254 pede macOS: build e testes no CI, config/histórico/senhas
no lugar padrão do sistema (Application Support, Keychain), atalhos com `Cmd` e artefato `.app`/`.dmg`
sem assinatura. Diferenças de plataforma devem ficar concentradas em `storage/` e `net/`, sem `#[cfg]`
espalhado. Não há SDK da Apple nas máquinas de desenvolvimento: a verificação real só acontece no CI.

Estado atual relevante:
* `storage/paths.rs` resolve XDG com fallback `~/.config`, `~/.local/share`, `~/.local/state`.
* `keyring` 4 com features padrão (`v1`) já seleciona `apple-native-keyring-store` (Keychain) no macOS;
  `detect_secret_store` não muda.
* Permissões 0600/0700 usam `std::os::unix`, que existe no macOS.
* Atalhos já usam `Modifiers::COMMAND` (vira `Cmd` no macOS); só os rótulos ("Ctrl+F", "F5") são fixos.
* Dependências com C/asm (`ring`, `aws-lc-sys`, `libsqlite3-sys` bundled) compilam com o Xcode CLT.

## Opções Consideradas
1. Caminhos: crate `directories`/`dirs` vs. tabela própria em `paths.rs` (std).
2. CI x86_64: runner `macos-13` (Intel, descontinuado) vs. `cargo check --target x86_64-apple-darwin`
   no mesmo runner `macos-latest` (arm64).
3. Empacotamento: `cargo-bundle`/`cargo-packager` vs. script `packaging/build-macos.sh` com
   `Info.plist` versionado, `sips`/`iconutil` (ícone) e `hdiutil` (`.dmg`), todos nativos do runner.

## Decisão Escolhida
* **Caminhos (std, sem dependência nova):** `paths.rs` continua honrando `XDG_*_HOME` quando
  definido (isola testes e respeita quem configurou de propósito); sem XDG, o fallback por SO é
  `~/Library/Application Support/crustdb` (config e dados) e `~/Library/Logs/crustdb` (estado/log)
  no macOS, e os caminhos XDG atuais no Linux. A função de resolução recebe o SO como parâmetro e é
  testada para os dois SOs em qualquer host; o único `#[cfg(target_os = "macos")]` escolhe o SO atual.
* **Segredos:** Keychain via `keyring` 4 já configurado; sem mudança de feature. `keyring_roundtrip`
  roda no job macOS do CI (o login keychain do runner é desbloqueado).
* **CI:** job `macos` em `macos-latest`: `cargo clippy --all-targets -D warnings`,
  `cargo nextest run --workspace` (testes `#[ignore = "docker"]` ficam só no Linux) e
  `cargo check --target x86_64-apple-darwin`. Entra em `ci-ok`.
* **Rótulos de atalho:** gerados por `egui::Context::format_shortcut` (mostra `⌘`/`Ctrl` conforme
  `ctx.os()`), testáveis no Linux com `ctx.set_os(OperatingSystem::Mac)`.
* **Empacotamento:** script próprio (opção 3), binário universal via `lipo`, `.dmg` sem assinatura,
  job no `release.yml`. Sem ferramenta nova.

### Consequências Positivas
* Nenhuma dependência nova; diferenças de SO isoladas em `storage/paths.rs`.
* Lógica de caminhos e rótulos de ambos os SOs testada também no Linux.

### Consequências Negativas / Trade-offs
* Runner macOS custa mais minutos de CI e só ele valida link, Keychain e janela real.
* x86_64 só é compilado (`check`), não testado; teste em Intel fica para quando houver relato.
* App sem assinatura: o Gatekeeper exige "Open Anyway" na primeira execução (assinatura é issue própria).
* Alguns atalhos colidem com o sistema no macOS (`Cmd+H` oculta o app, `Cmd+Tab`, `Cmd+Space`);
  remapeamento fica na fatia de atalhos.
