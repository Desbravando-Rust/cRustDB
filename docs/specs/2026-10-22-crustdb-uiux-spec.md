# UI/UX Spec: cRustDB v1.x (melhorias)

Spec de melhorias de UI/UX sobre o app v1 já implementado. Derivada do código real (HEAD `d859b5b`, `crates/crustdb-app/src/`) e da spec v1 (`2026-10-17-crustdb-spec.md`). Issue #33.

Leitor alvo: agente Claude Code que vai implementar, uma issue por grupo (ver §7). Texto da UI em inglês; este documento em pt-BR. Caminhos `arquivo:linha` são relativos a `crates/crustdb-app/src/` e foram conferidos contra o HEAD acima.

---

## 1. Visão geral

- **Objetivo:** consistência visual, descoberta de ações e feedback de estado, sem novas features de banco.
- **Relação com a spec v1:** o wireframe (§4.11) e os atalhos fixos (§4.9) continuam a referência; §11 deixa "cores exatas e layout fino" com o builder, e é nessa margem que este documento atua.
- **Restrições herdadas (respeitadas):** D17 (nenhuma ação de copiar célula/linha, texto do row detail não selecionável) e §10 (tema claro, atalhos customizáveis, cores/tags de conexão ficam fora). O que colide está em §6 (Backlog v2).
- **Prioridades:** P1 = inconsistência ou bug visível, barato; P2 = melhora clara de usabilidade; P3 = polimento.
- **Aceite:** testável com `egui_kittest` (padrão em `ui/home.rs:116-196`: `Harness`, `get_by_label`, `click`) quando couber; caso contrário, teste de unidade do estado ou verificação manual descrita.

## 2. Já entregue e pendente

| Issue | Entrega | Onde |
|---|---|---|
| #32 | Logo na home | `ui/home.rs:28-32` |
| #35 | Duplo clique conecta | `ui/home.rs:59` |
| #35 | Badge "Active" + botão "Open" | `ui/home.rs:79-82` |
| #35 | Cor por engine (azul PG, laranja MySQL) | `ui/home.rs:8-17` |
| #34 (pendente) | Propaganda do livro na home | só reservar espaço (U-H6) |

## 3. Princípios

1. **Uma cor, um significado:** verde = ok/ativo, âmbar = atenção/transação, vermelho = erro. Definidos uma vez em `ui/theme.rs`.
2. **Descoberta:** todo atalho tem botão visível ou tooltip com o atalho.
3. **Feedback de carregamento:** toda espera visível mostra `egui::Spinner` + texto.
4. **Nunca bloquear a UI thread:** nada neste documento adiciona IO síncrono; pickers e consultas seguem em thread/tokio (`ui/picker.rs:30`).
5. **Ações destrutivas** se distinguem das neutras.

## 4. Inventário de telas

Colunas: ID | Problema | Evidência | Proposta | Prio | Aceite.

### 4.1 Home (`ui/home.rs`, 197 linhas incl. testes)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-H1 | Nome duplicado: logo (alt "cRustDB logo") e heading "cRustDB" | `ui/home.rs:28-32`, `:34` | Manter só o logo; título da janela já traz o nome | P3 | kittest: `query_by_label("cRustDB")` não encontra heading |
| U-H2 | Lista sem busca nem navegação por teclado | `ui/home.rs:49` (loop simples) | Campo "filter" acima da lista; setas move seleção, Enter conecta | P2 | kittest: digitar "pg" deixa só conexões com "pg"; Enter dispara `Intent::Connect` |
| U-H3 | Estado vazio é só texto fraco | `ui/home.rs:47` | Bloco centralizado com botões "New" e "Import URL" | P2 | kittest: sem conexões, clicar "New" emite `Intent::NewConnection` |
| U-H4 | Erro de conexão numa linha solta, sem retry | `ui/home.rs:98-99` | Ícone de aviso + mensagem junto ao item; botão "Retry" reaproveita `Intent::Connect` | P2 | kittest: workspace `Failed` mostra "Retry"; clique emite `Connect` |
| U-H5 | Delete ao lado de Edit sem destaque destrutivo | `ui/home.rs:65`, `:72` | Separar Delete (cor de erro no hover) | P3 | Unidade: função pura `delete_color(hovered, visuals)` devolve `error_fg_color` só em hover; revisão manual: ordem Edit, Delete (já é a atual, `right_to_left`) preservada |
| U-H6 | Espaço para a propaganda do #34 | `ui/home.rs:47-49` (fim da lista) | Reservar rodapé discreto fixo (altura ≤ 48 px), sem conteúdo nesta issue | P3 | Revisão manual: rodapé não empurra a lista |

### 4.2 Form de conexão (`ui/connection_form.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-F1 | Modo TLS mostrado em Debug ("VerifyCa") | `ui/connection_form.rs:123` | `Display` legível ("Verify CA"), compartilhado com o tooltip (U-S2) | P1 | kittest: combo mostra "Verify CA", não "VerifyCa" |
| U-F2 | Paths (CA, chave SSH) só em texto livre | `ui/connection_form.rs:24-32` (`path_edit`), `:141-142`, `:213-216` | Botão "Browse" ao lado, usando `Picker` existente (`ui/picker.rs:30`) | P2 | Unidade: `Pick` novo devolve o path ao draft; sem Browse não há regressão do campo texto |
| U-F3 | Linha vazia por campo mesmo sem erro | `ui/connection_form.rs:63-64`, `:85-86`, `:91-92`, `:147-148`, `:153-154` | `error_for` só abre linha quando há erro, ou erro na mesma célula do campo | P1 | kittest: form válido tem altura menor que com erro em todos os campos |
| U-F4 | Porta `DragValue` sem range: arrastar altera sem querer | `ui/connection_form.rs:89`, `:177` | `.range(1..=65535)` | P1 | kittest/unidade: valor 0 ou 70000 é limitado |
| U-F5 | Tudo numa lista só | `ui/connection_form.rs:57-164` (campos), `:172` (SSH) | `CollapsingHeader`: Basic aberto; TLS, Advanced (page size, row limit, threshold `:151-161`) e SSH recolhidos | P2 | kittest: "Advanced" começa fechado e abre ao clicar; erro de validação num campo recolhido o abre |
| U-F6 | Sem botão primário nem Enter/Esc | `ui/connection_form.rs:248-258` | "Connect" em destaque; Enter = Connect, Esc = Cancel (sem Enter dentro de combo aberto) | P2 | kittest: Esc emite `FormCancel`; Enter com form válido emite `FormConnect` |
| U-F7 | Teste de conexão só com texto "Testing..." | `ui/connection_form.rs:237` | `Spinner` + texto (ver §5.3) | P2 | kittest: durante `TestResult::Running` existe o rótulo "Testing" |

### 4.3 Sidebar (`ui/sidebar.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-B1 | Setas unicode e nenhum ícone por tipo | `ui/sidebar.rs:40` | Ícones Phosphor (`icon::` já usado em `ui/home.rs:65`) para schema/tabela/view/função | P2 | kittest: nó de tabela e de view renderizam rótulos distintos de ícone |
| U-B2 | Folha: clique simples não faz nada, só duplo abre | `ui/sidebar.rs:53-55` | Clique seleciona; Enter abre o selecionado; tooltip "Double-click to open" | P2 | kittest: Enter com folha selecionada emite `OpenObject` |
| U-B3 | Sem menu de contexto (Open/Refresh) | `context_menu` não é usado em `ui/` (busca vazia) | `response.context_menu` com Open e Refresh; Refresh hoje só via F5 (`keymap.rs:45`) | P2 | kittest: botão direito mostra "Refresh" e emite o mesmo intent do F5 |
| U-B4 | "New query" sem ícone nem atalho; wireframe §4.11 mostra `[+]` | `ui/sidebar.rs:24` | Botão com ícone `+` e tooltip "New query (Ctrl+T)" | P3 | kittest: `get_by_label("New query")` continua clicável |
| U-B5 | Combo de workspace sem status nem desconectar | `ui/sidebar.rs:12-23` | Bolinha de status por item e ação "Disconnect" | P3 | Unidade: `status_color(&Status) -> Color32` (padrão `tls_icon`, `ui/status_bar.rs:17`) devolve `WARN` para `Connecting`, `OK` para `Online`, `ERR` para `Failed` |
| U-B6 | Carregamento só com sufixo " …" | `ui/sidebar.rs:43` | `Spinner` pequeno ao lado do nó | P3 | kittest: nó `loading` mostra o texto de carregamento (como §5.3); Spinner em si não é verificável via AccessKit |

### 4.4 Tab bar (`ui/tab_bar.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-T1 | Abas de tabela e query indistintas, sem tooltip | `ui/tab_bar.rs:13`, `:25` (só texto) | Ícone por tipo; tooltip com schema.tabela ou caminho do arquivo | P2 | kittest: aba de query e de tabela têm ícones diferentes |
| U-T2 | Estado da aba é sufixo de texto sem cor | `ui/tab_bar.rs:19-24` | Ponto/ícone colorido: âmbar = tx aberta, vermelho = aborted, neutro = dirty | P1 | Unidade: `tx_marker(TxState, dirty) -> (&str, Color32)` (padrão `tls_icon`, `ui/status_bar.rs:17`) devolve `WARN` para `Open`, `ERR` para `Aborted` |
| U-T3 | X sempre visível; sem overflow com muitas abas | `ui/tab_bar.rs:27-38` | `ScrollArea::horizontal`; X só em hover ou aba ativa | P2 | kittest: 30 abas não alargam a janela; botão de scroll/overflow acessível |

### 4.5 Aba de tabela (`ui/grid.rs`, `ui/structure.rs`, `ui/filter_bar.rs`, `ui/row_detail.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-G1 | Rodapé só tem `<` `>`, rótulo, Stop/Retry; faltam refresh, CSV e row detail do wireframe | `ui/grid.rs:93-121`; atalhos em `keymap.rs:45`, `:58` | Botões com ícone e tooltip (Refresh F5, Export CSV Ctrl+Shift+E, Row detail Space) e seletor de page size | P2 | kittest: clicar "Export CSV" emite o mesmo intent do atalho |
| U-G2 | `<` e `>` crus, sem tooltip | `ui/grid.rs:96`, `:102` | Ícones de seta com tooltip "Previous page (Ctrl+[)" / "Next page (Ctrl+])" | P3 | kittest: tooltip contém o atalho |
| U-G3 | Filtro só por Ctrl+F, sem botão | `ui/grid.rs:122` (`filter_open`), `keymap.rs:46` | Botão "Filter" na barra superior da tabela | P2 | kittest: clicar "Filter" abre o painel `filter_bar` |
| U-G4 | "−" e "+ Filter" em texto | `ui/filter_bar.rs:39`, `:48` | Ícones Phosphor com tooltip | P3 | kittest: tooltips "Remove condition" / "Add condition" |
| U-G5 | Aviso "unordered" amarelo cru, sem explicação | `ui/grid.rs:117-118` | Usar `warn_fg_color` + tooltip "No primary key: row order is not guaranteed" | P1 | kittest: hover em "unordered" mostra tooltip |
| U-G6 | Structure é texto monoespaçado corrido | `ui/structure.rs:23`, `:43`, `:61` (3 `monospace`) | Tabelas (`egui_extras::TableBuilder` (já é dependência, `crates/crustdb-app/Cargo.toml:18`)) por seção; seção vazia mostra "none" | P2 | kittest: seção Columns tem cabeçalho "Name/Type/Nullable"; índice vazio mostra "none" |
| U-G7 | Título do row detail junta coluna e tipo num texto | `ui/row_detail.rs:60` | Coluna em negrito e tipo em `weak` separado | P3 | kittest: rótulos de coluna e tipo são nós distintos |

### 4.6 Aba de query (`ui/editor.rs`, `ui/results.rs`, `ui/history_panel.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-Q1 | Toolbar numa única linha transborda em janela estreita | `ui/editor.rs:124` (Run), `ui/results.rs:14-50` (Stop/Tx/db), `ui/editor.rs:264-279` (arquivo) | `horizontal_wrapped` ou agrupar em 2 linhas: execução e arquivo | P2 | kittest: com largura 500 px nenhum botão fica fora do `Ui` |
| U-Q2 | Atalhos só em Run/Run all; Stop/Open/Save/History sem | `ui/editor.rs:126`, `:138` vs `:270-273`; `ui/results.rs:15` | Tooltip com atalho em todos (`keymap.rs:39-59`) | P1 | kittest: hover em "Save" mostra "Ctrl+S" |
| U-Q3 | Begin/Commit/Rollback sempre habilitados | `ui/results.rs:21-29`, estado em `tab.tx` `:30` | Begin só com `Idle`; Commit/Rollback só com `Open`/`Aborted` | P1 | kittest: `TxState::Idle` desabilita Commit e Rollback |
| U-Q4 | Cores de transação divergem da status bar | `ui/results.rs:32-33` vs `ui/status_bar.rs:71-72` | Tokens únicos (§5.1) | P1 | Unidade: ambos usam `theme::TX_OPEN` |
| U-Q5 | Campo `[limit 1000]` do wireframe não existe | wireframe §4.11 vs `ui/editor.rs:124-153` | Campo numérico (default do config) ao lado de Run | P3 | kittest: alterar o limit muda o `RunQuery` emitido |
| U-Q6 | "No result" é estado vazio pobre; erro só em `LIGHT_RED`, sem ícone | `ui/results.rs:85`, `:100` | Mensagem guia ("Run a query with Ctrl+Enter") e ícone de erro | P2 | kittest: aba vazia contém "Ctrl+Enter" |
| U-Q7 | Histórico: erro com prefixo "! "; duração só em tooltip | `ui/history_panel.rs:21`, `:24` | Ícone de erro e duração visível em `weak` | P3 | kittest: entrada `ok=false` tem ícone, não "! " |

### 4.7 Diálogos modais (`ui/confirm_dialog.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-D1 | Sem Enter/Esc e sem foco no campo de senha | `ui/confirm_dialog.rs:66-70` (`TextEdit` sem `request_focus`); `Key::Escape` só em `state/autocomplete.rs:203` | `request_focus` ao abrir; Esc = Cancel; Enter = ação primária (exceto destrutivas sem digitar a senha) | P1 | kittest: Esc emite o intent de Cancel; campo de senha nasce focado |
| U-D2 | Botão destrutivo igual ao Cancel | `ui/confirm_dialog.rs:79` (Execute), `:93` (Rollback and close), `:107` (Discard) | Cor de erro no botão destrutivo; Cancel neutro e último | P1 | Unidade: `button_color(DialogAction, &Visuals) -> Option<Color32>` devolve `error_fg_color` para Execute, Rollback and close e Discard, e `None` para Cancel |
| U-D3 | Sem backdrop modal: `Window` ancorado deixa a UI atrás clicável | `ui/confirm_dialog.rs:48-51` | `Modal` do egui (ou camada que bloqueia input) | P2 | kittest: clicar fora do diálogo não aciona botões do fundo |

### 4.8 Status bar (`ui/status_bar.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-S1 | "TX open"/"TX aborted" em texto, diferente dos outros itens com ícone | `ui/status_bar.rs:71-72` | Ícone Phosphor + cor, como o cadeado e o SSH (`:19-21`, `:67`) | P2 | kittest: item de tx tem tooltip "Transaction open" |
| U-S2 | Tooltip TLS em Debug | `ui/status_bar.rs:64` | Mesmo `Display` de U-F1 | P1 | kittest: tooltip "TLS mode: Verify CA" |

### 4.9 Janela e tema (`main.rs`, `ui/theme.rs`)

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-W1 | Viewport só define título: sem tamanho inicial, mínimo e ícone | `main.rs:58` | `with_inner_size`, `with_min_inner_size`, `with_icon` (a partir de `assets/imgs/logo2.png`) | P2 | Unidade: `NativeOptions` expõe tamanho mínimo; manual: ícone no gerenciador de janelas |

## 5. Transversais

### 5.1 Paleta e tokens (P1)

Hoje as mesmas cores aparecem duplicadas e com variantes diferentes por tela.

| ID | Problema | Evidência | Proposta | Prio | Aceite |
|---|---|---|---|---|---|
| U-X1 | Verde duplicado | `ui/home.rs:8` (`ACTIVE`) e `ui/status_bar.rs:11` (`GREEN`), mesmo RGB (90,190,110) | Constantes únicas em `ui/theme.rs` (hoje só instala fonte e spacing, `:4-18`): `OK`, `WARN`, `ERR` | P1 | Unidade: `home` e `status_bar` importam de `theme`; `rg "from_rgb\(90, 190, 110\)"` retorna 1 ocorrência |
| U-X2 | Transação com cores divergentes | `ui/results.rs:32` `LIGHT_GREEN` vs `ui/status_bar.rs:71` `AMBER` | Transação aberta = `WARN` nos dois | P1 | Ver U-Q4 |
| U-X3 | Erro sem padrão | `LIGHT_RED` em `ui/grid.rs:158`, `ui/results.rs:33`, `:100`; `error_fg_color` em `ui/structure.rs:10`, `ui/grid.rs:112`, `ui/connection_form.rs:36`, `ui/home.rs:99`; `RED` em `ui/status_bar.rs:13` | Tudo via `visuals().error_fg_color` ou `theme::ERR` | P1 | `rg "LIGHT_RED" ui/` retorna 0 |
| U-X4 | Aviso com `Color32::YELLOW` em vez do visual do tema | `ui/grid.rs:118` | `visuals().warn_fg_color` | P1 | Unidade: `unordered_color(&Visuals)` devolve `warn_fg_color`; `rg "Color32::YELLOW" ui/` retorna 0 |
| U-X5 | Duas paletas de highlight SQL | editor `ui/editor.rs:52-60` vs DDL `ui/grid.rs:165-171` | Função única `theme::token_color(TokenKind)` | P2 | Unidade: DDL e editor produzem a mesma cor para `Keyword` |

### 5.2 Ícones (P3)

Padronizar Phosphor (`egui_phosphor::regular`, já usado em `ui/tab_bar.rs:35`) em vez de glifos unicode (`ui/sidebar.rs:40`) e texto (`ui/filter_bar.rs:39`). Aceite: `rg '"[▾▸<>−]"' ui/` sem ocorrências fora de testes.

### 5.3 Indicadores de carregamento (P2)

`egui::Spinner` não é usado em nenhum lugar de `ui/`. Textos atuais: "Loading…" `ui/structure.rs:15`, "Loading DDL…" `ui/grid.rs:185`, "Testing..." `ui/connection_form.rs:237`, "Connecting..." `ui/home.rs:86`, sufixo " …" `ui/sidebar.rs:43`. Proposta: helper `ui::loading(ui, "text")` = Spinner + label, usado nos cinco pontos. Aceite: kittest procura o texto em cada estado e a ausência de bloqueio (o frame renderiza com a tarefa pendente).

### 5.4 Teclado (P2)

Atalhos são fixos (`keymap.rs:39-59`, spec §4.9). Pendências: Enter/Esc em diálogos e form (U-D1, U-F6), setas e Enter na home e na sidebar (U-H2, U-B2). Nenhum atalho novo global; customização fica em §6.

## 6. Backlog v2 (fora do v1 por D17 e §10)

| Item | Motivo |
|---|---|
| Tema claro / alternância de tema | §10 (tema claro); `ui/theme.rs:6` fixo em Dark |
| Atalhos customizáveis | §10 |
| Cores/tags por conexão (ex.: "prod" vermelho) | §10; hoje só a cor por engine (#35) |
| Selecionar e copiar texto de célula, linha, row detail ou erro | D17 (sem clipboard); `ui/row_detail.rs:60` e `ui/grid.rs:183` (DDL selecionável) já são o limite |
| Botão "Copy" em erros e no DDL | D17 |
| Menu de contexto "Copy cell" | D17 |

## 7. Ordem sugerida de implementação (uma issue por grupo)

1. **P1 paleta e tokens:** U-X1 a U-X4, U-Q4, U-T2, U-S2 (depende de nada; U-X4 tem aceite próprio).
2. **P1 form e diálogos:** U-F1, U-F3, U-F4, U-D1, U-D2.
3. **P1 query:** U-Q2, U-Q3, U-G5.
4. **P2 carregamento e navegação:** §5.3, U-H2 a U-H4, U-B1 a U-B3, U-T1, U-T3.
5. **P2 tabela e layout:** U-G1, U-G3, U-G6, U-Q1, U-Q6, U-F2, U-F5, U-F6, U-F7, U-D3, U-S1, U-W1, §5.4, U-X5.
6. **P3 polimento:** demais itens P3 (inclui U-H6 depois de #34).

Cada issue: teste kittest do Aceite primeiro, depois a mudança; gates do `CLAUDE.md` do repo.
