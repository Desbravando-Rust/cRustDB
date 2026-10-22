# Technical Spec: cRustDB v1

Client desktop Linux para MySQL/MariaDB e PostgreSQL, escrito em Rust. Escopo v1: explorar dados em grid **somente leitura** e executar **SQL raw** com proteções contra escrita acidental. Sem features avançadas (edição de dados, diagramas, import, backup, IA).

Leitor alvo: agente Claude Code que vai implementar. Regras de engenharia: performance mensurável, SOLID, Clean Code, padrões de projeto apenas onde há ≥2 implementações reais (zero abstração especulativa). Identificadores, UI e código em inglês.

---

## 1. Technical overview

- **Complexidade:** complex (greenfield, 2 drivers, GUI, async, segurança).
- **Stack:** Rust stable mais recente (edition 2024) · `eframe`/`egui` (UI) · `tokio` (IO) · `tokio-postgres` · `mysql_async` · `sqlparser` (split/classificação/tokens) · `rustls` · `russh` (SSH) · `keyring` (Secret Service) · `csv` · `serde`/`serde_json` · `thiserror` · `tracing`.
- **Partes:** workspace Cargo com 4 crates: domínio puro (`core`), 2 drivers, app (UI + storage + runtime).
- **Princípio de dados:** tudo que o banco devolve é tratado como **texto** (protocolo texto / cast `::text`) + metadado de tipo da coluna. Evita mapa de decode por tipo, preserva decimais exatos, permite streaming.

## 2. Architecture impact

Tudo novo. Direção de dependência (o compilador garante):

```
crustdb-app ──► crustdb-postgres ──► crustdb-core
     │      └─► crustdb-mysql    ──► crustdb-core
     └──────────────────────────────► crustdb-core
```

- `crustdb-core`: tipos de domínio, traits `Driver`/`Session`/`Introspector`/`Dialect`, split/classificação/realce/autocomplete de SQL, builder de query do grid, política de segurança. **Sem IO, sem tokio, sem egui.**
- `crustdb-postgres` / `crustdb-mysql`: implementações dos traits. Não conhecem SSH nem UI (recebem host/porta já resolvidos).
- `crustdb-app`: egui, runtime tokio, atores de sessão, túnel SSH, persistência (config, keyring, histórico), métricas, export CSV, binário `crustdb`.

### Modelo de concorrência

```
 UI thread (eframe loop, nunca bloqueia)
   │  Command (tokio mpsc, send não-bloqueante)        ▲ Event (std mpsc) + ctx.request_repaint()
   ▼                                                   │
 tokio runtime (thread própria, multi-thread, 2 workers)
   ├─ WorkspaceActor (1 por conexão aberta): sessões Metadata (1 por database no PG), túnel SSH, ping 60 s
   ├─ EditorActor (1 por aba de query): sessão Editor + cursor pausado + estado de transação
   ├─ DataActor (1 por aba de tabela): sessão Data read-only, páginas, Stop
   └─ ExportTask (1 por export de tabela): sessão Export própria, CSV em streaming (export de query roda no EditorActor, D26)
```

- Padrão **Actor**: cada sessão tem um único dono; comandos processados em ordem; sem `Mutex` em sessão.
- Todo `Command` carrega `RequestId` (u64 monotônico). UI descarta `Event` com id obsoleto (ex.: usuário paginou de novo).
- Linhas chegam em lotes (`Event::Rows`, até 500 linhas por evento), nunca 1 evento por linha.

## 3. Decisions and assumptions

Fonte: **U** = decidido pelo usuário · **P** = proposto e aprovado · **A** = assumido (revisar).

| # | Decisão | Escolha | Por quê | Fonte |
|---|---|---|---|---|
| D1 | Plataforma | Linux x86_64 (X11 e Wayland) | — | U |
| D2 | UI | `egui`/`eframe`, backend `glow` | Rust puro, estado testável sem GUI, tabela virtualizada | P / backend A |
| D3 | Tema / idioma | Escuro fixo; UI em inglês | — | U |
| D4 | Atalhos | Fixos (§4.9); customização = v2 | — | U / mapeamento A |
| D5 | Bancos | PostgreSQL ≥ 12; MySQL ≥ 5.6.5; MariaDB ≥ 10.0.5 | MySQL 5.6.3 = `EXPLAIN UPDATE/DELETE`; 5.6.5 = `TRANSACTION READ ONLY` | U (pisos) / P |
| D6 | Piso real | Se `mysql_async` falhar no piso, ele sobe para a menor versão que passar na matriz de testes; registrar no README | Não há lista oficial de versões testadas | P |
| D7 | Drivers | `tokio-postgres` (+ `tokio-postgres-rustls`) e `mysql_async` (feature rustls) | Protocolo texto, streaming, cancel nativo | P |
| D8 | Conexões salvas | `$XDG_CONFIG_HOME/crustdb/connections.json`, sem segredos | — | U / local P |
| D9 | Segredos | Keyring via Secret Service; sem Secret Service → não salva, pede a cada conexão; nunca em disco | "O mais seguro" | U / P |
| D10 | TLS | `rustls`; modos `disable`/`require`/`verify-ca`/`verify-full`; padrão `verify-full`; sem `prefer` | `prefer` cai para texto puro silenciosamente | P |
| D11 | SSH | `russh`, port forward local em `127.0.0.1:<efêmera>`; auth senha, chave+passphrase, agent; `known_hosts` estrito | — | U (auth) / P |
| D12 | Conexão por URL | Import de `postgres://`, `postgresql://`, `mysql://`, `mariadb://` | — | U |
| D13 | Sessões | 1 janela; N conexões (workspaces). Sessões **read-only no nível do banco**: Metadata (sidebar, autocomplete, estimativas; 1 por database no PG), Data (1 por aba de tabela, aberta lazy, com Stop no rodapé durante o carregamento), Export de tabela (1 por export). Sessão Editor: 1 por aba de query. Cancel via token (PG) / conexão efêmera `KILL QUERY` (MySQL) | Transação pertence à sessão; página lenta não trava sidebar nem outras abas | P (+ builder check) |
| D14 | Grid | Somente leitura; sort múltiplo; filtros; estrutura; DDL; row detail; export CSV | — | U |
| D15 | Paginação | `LIMIT/OFFSET`, página padrão 300, configurável por conexão; busca `limit+1` para saber se há próxima | Teto: OFFSET profundo é lento → keyset na v2 | U (configurável) / P |
| D16 | Contagem | Só estimativa: `pg_class.reltuples` / `information_schema.TABLES.TABLE_ROWS`; nunca `COUNT(*)` | — | U |
| D17 | Clipboard | Nenhuma ação de copiar célula/linha na v1 | — | U |
| D18 | Editor | Realce (tokenizer do `sqlparser`), autocomplete, executar statement sob cursor / seleção / tudo, cancelar, limite de linhas + "Load more", histórico, abrir/salvar `.sql` | — | U / modos P |
| D19 | Vários statements | Sem várias abas: grid mostra o **último** statement que retornou linhas; aba Messages lista todos | — | U |
| D20 | Transações | Autocommit padrão; botões Begin/Commit/Rollback por aba | — | U |
| D21 | Segurança SQL | Confirmação simples para destrutivos ou multi-linha; senha quando a estimativa passar de X (padrão 1000, configurável por conexão, mínimo 1); estimativa via `EXPLAIN` | — | U |
| D22 | Statement não reconhecido pelo parser | Confirmação simples | Fail-safe sem bloquear `CALL`/`DO` | A |
| D23 | `EXPLAIN` falhou (estimativa desconhecida) em DML | Tratado como acima de X → senha | Fail-safe | A |
| D24 | `DROP TABLE`/`TRUNCATE` | Confirmação; senha se a estimativa de linhas da tabela alvo passar de X | Coerente com "impacto > X linhas" | A |
| D25 | Conexão sem senha (trust/socket) | "Digitar senha" vira "digitar o nome do database" | Mantém atrito equivalente | A |
| D26 | Export | Só CSV. Aba de tabela: tabela inteira com filtros+sort ativos, em sessão Export própria. Aba de query: resultado completo do último statement (linhas carregadas + restante drenado do cursor, **sem re-executar**), rodando **dentro do EditorActor** (a aba fica ocupada, com progresso e Cancel). Células já carregadas e truncadas em 1 MiB (§4.4) saem truncadas; DML com `RETURNING` exporta só as linhas guardadas (D29). Nos dois casos a aba Messages avisa | Re-executar poderia repetir escrita; o cursor só existe na sessão do editor | U / mecanismo A |
| D27 | PG: leitura no editor | Statements de leitura (`SELECT`/`WITH`/`VALUES`/`TABLE`) via cursor de servidor (`DECLARE … NO SCROLL CURSOR` + `FETCH n`), dentro da transação do usuário se houver, senão em transação implícita fechada ao final | Memória limitada, "Load more" determinístico | P |
| D28 | MySQL: leitura no editor | `query_iter` pausado (backpressure do socket); descartar = `KILL QUERY` + drenar | Sem buffer ilimitado | P |
| D29 | Escrita no editor | Nunca cancelar no meio para descartar linhas: drena tudo, guarda as N primeiras | Cancelar no meio desfaria a escrita | P |
| D30 | Ícones | Fonte `egui-phosphor` | egui não traz ícones | A |
| D31 | Métricas | CPU% e RSS de `/proc/self/stat` e `/proc/self/status` (VmRSS), 1 Hz; `USER_HZ = 100` | Sem dependência, sem `unsafe` | P |
| D32 | Distribuição | AppImage + tarball (binário + `.desktop` + ícone) | — | P |
| D33 | Histórico | JSONL por conexão em `$XDG_DATA_HOME/crustdb/history/`, 0600, teto 10.000 entradas | — | P / teto A |
| D34 | Estrutura inclui FKs | Colunas, índices, foreign keys | Barato, mesmo catálogo | A |
| D35 | Abas não salvas | Conteúdo de aba sem arquivo não persiste entre sessões | Não pedido | A |
| D36 | Workspace multi-crate | 4 crates (§2) | Driver novo = crate novo (OCP); core testável sem GUI | U |
| D37 | Ciclo do cursor pausado | Só o **último statement do lote** pode ficar pausado (Load more/export). Statements anteriores com linhas: lê até `editor_row_limit` e fecha. Qualquer comando novo na sessão (Run, Begin/Commit/Rollback, troca de database, EXPLAIN da política) finaliza antes o cursor pausado (PG: `CLOSE` + `COMMIT` da transação implícita, ou só `CLOSE` dentro de transação do usuário; MySQL: `KILL QUERY` + drenar). Cursor ocioso por 5 min é finalizado sozinho, com aviso em Messages | Transação implícita aberta segura snapshot/locks; a conexão MySQL fica bloqueada | A |
| D38 | PG: tipos das colunas no editor | Antes de executar, `prepare` (Describe) do statement para obter os OIDs das colunas → `TypeCategory`; execução continua em protocolo texto (`simple_query_raw`/cursor). `prepare` falhou → todas as colunas `TypeCategory::Other` | `SimpleColumn` só traz o nome, sem tipo | A |
| D39 | Versão mínima de TLS | Só `rustls` (TLS ≥ 1.2). Servidor sem TLS 1.2 (ex.: MySQL 5.6 com yaSSL) → erro claro "server does not support TLS 1.2+; use an SSH tunnel or disable TLS". Sem `native-tls` | TLS 1.0/1.1 está quebrado; coerente com "o mais seguro" | U (aprovado) |
| D40 | Widget do editor | `egui::TextEdit` + `layouter` com realce memoizado (sem widget próprio na v1). Se a meta de digitação do §7 falhar na medição, o realce é desligado acima de N linhas (N medido, registrado no README) em vez de criar widget virtualizado | Widget próprio é um bloco grande de trabalho; fica para a v2 se necessário | U (aprovado) |

## 4. Requirements

### 4.1 Conexões
- **Home** (nenhum workspace aberto): lista de conexões salvas com Connect / Edit / Delete (com confirmação) / New / Import URL.
- **Formulário**: `name`, `driver` (`PostgreSQL` | `MySQL/MariaDB`), `host`, `port` (padrão 5432/3306), `user`, `password` + checkbox "Save in keyring", `database` (opcional), `tls.mode`, `tls.ca_path` (opcional; vazio = raízes do sistema via `rustls-native-certs`), `ssh` (toggle; `host`, `port` 22, `user`, `auth`: Password | Key file (+ passphrase) | Agent), `page_size` (300), `editor_row_limit` (1000), `safety_threshold` (1000). Botões Test / Save / Connect.
- **Validação** (fronteira de confiança): `name` não vazio e único; `port` 1–65535; `page_size` 10–10.000; `editor_row_limit` 10–100.000; `safety_threshold` ≥ 1; arquivo de chave/CA existe e é legível.
- **URL**: parse preenche o formulário; `sslmode`/`ssl-mode` mapeados para os modos de D10 (`prefer`/`allow` → `require` com aviso); senha da URL só vai ao keyring se o checkbox estiver marcado.
- **Rebaixar TLS** abaixo de `verify-full` mostra aviso no formulário e ícone âmbar no status bar.
- **TLS via SSH**: verificação de hostname usa o host real do banco, não `127.0.0.1` (PG: `Config::host` = host real + `hostaddr` = 127.0.0.1; MySQL: `SslOpts::with_danger_tls_hostname_override(Some(host real))`, existente no `mysql_async` 0.37). Nunca rebaixa a verificação sozinho.
- **SSH host key**: desconhecida → diálogo com fingerprint SHA256 (Accept & save / Reject), salva em `$XDG_CONFIG_HOME/crustdb/known_hosts` (formato OpenSSH). Também lê `~/.ssh/known_hosts`, sem escrever. Chave divergente → erro bloqueante, sem override.
- **Timeouts**: TCP/TLS 10 s; SSH 10 s; queries sem timeout (usuário cancela).
- **Keep-alive**: ping a cada 60 s na sessão Metadata; latência no status bar. Queda: Metadata reconecta sozinha (backoff 1, 2, 4, 8 s, máx 30 s); Editor informa "Connection lost — open transaction was rolled back by the server" e reconecta no próximo Run.

### 4.2 Sidebar (árvore de objetos, carga lazy, cache por nó, refresh F5)

| Nível | PostgreSQL | MySQL/MariaDB |
|---|---|---|
| 1 | Databases (`pg_database`, sem templates, `datallowconn`) | Databases (`information_schema.SCHEMATA`) |
| 2 | Schemas (exceto `pg_toast`, `pg_temp_*`, `pg_toast_temp_*`) | — |
| Pastas | Tables (relkind `r`,`p`), Views (`v`), Materialized Views (`m`), Functions (`prokind f`), Procedures (`p`), Triggers (`NOT tgisinternal`), Sequences (`S`), Types (enum/composite/domain sem tabela) | Tables, Views, Functions, Procedures, Triggers |

- Campo de busca no topo filtra nós já carregados (substring, case-insensitive).
- Expandir outro database no PG abre nova sessão Metadata para ele (lazy).
- Duplo clique: tabela/view/matview → aba de dados; demais → aba DDL.

### 4.3 Aba de tabela (Data | Structure | DDL)
- **Data**: grid virtualizado. Rodapé: `‹ Prev`, `Next ›`, `Rows 301–600 of ~1.2M` (sem filtro: estimativa; com filtro: só o intervalo), input de page size (vale para a sessão), Refresh, Export CSV, toggle Row detail.
- **Sort**: clique no cabeçalho alterna asc → desc → nenhum; Shift+clique adiciona/alterna na ordenação múltipla; cabeçalho mostra seta + ordinal. Sem sort explícito: ordena pela PK se existir (estável entre páginas); sem PK, sem `ORDER BY` (aviso discreto "unordered").
- **Filtros** (Ctrl+F abre a barra): linhas `[column ▾] [operator ▾] [value] [×]`, `+`, Apply (Enter), Clear. Combinação **AND**. Operadores por categoria de tipo:

| Categoria | Operadores |
|---|---|
| Numérico / data / hora | `=` `!=` `<` `>` `<=` `>=` `IN` `NOT IN` `BETWEEN` `IS NULL` `IS NOT NULL` |
| Texto / UUID / JSON / outro | os acima + `contains` `not contains` `starts with` `ends with` `LIKE` `NOT LIKE` (+ `ILIKE` só PG) |
| Booleano | `=` `!=` `IS NULL` `IS NOT NULL` |
| Binário | `IS NULL` `IS NOT NULL` |

  `contains`/`starts with`/`ends with` são case-insensitive nos dois bancos (PG `ILIKE`; MySQL `LOWER(CAST(col AS CHAR)) LIKE LOWER(?)`), com `%`, `_` e `\` do valor escapados. `IN` aceita lista separada por vírgula. Valores **sempre** bound como parâmetro texto (PG: `query_typed_raw` com `Type::TEXT` e `CAST($n AS <tipo da coluna>)`; MySQL: statement preparado). Valor inválido para o tipo → erro do servidor exibido na barra de filtro.
- **Structure**: tabelas Columns (name, type, nullable, default, comment, PK), Indexes (name, columns, unique, primary, method), Foreign keys (columns → table(columns), on update/delete).
- **DDL**: texto realçado, somente leitura. MySQL: `SHOW CREATE …`. PG: `pg_get_viewdef`, `pg_get_functiondef`, `pg_get_triggerdef`, `pg_get_indexdef`, `pg_get_constraintdef`; tabela, sequence e type montados a partir do catálogo.
- **Row detail** (painel lateral; toggle Space com foco no grid): lista vertical coluna → tipo → valor completo; JSON formatado; binário em hex dump (primeiros 4 KiB + tamanho total); texto não selecionável (D17).

### 4.4 Exibição de células
- `NULL`: texto `NULL` cinza itálico (diferente de string vazia).
- Texto truncado em 256 caracteres no grid, com `…`; valor completo no row detail.
- Binário: `(binary 2.3 KB)` + hex dos primeiros 8 bytes. PG: `bytea` chega como `\x…`; MySQL: colunas com charset binário (63).
- JSON compacto no grid, formatado no row detail.
- Datas/horas: texto exato do servidor, sem conversão de fuso. MySQL em statement preparado (valores binários): formatar conforme o tipo da coluna (`DATE` → `YYYY-MM-DD`; `DATETIME`/`TIMESTAMP` → `YYYY-MM-DD HH:MM:SS[.ffffff]`; `TIME` → `[-]HHH:MM:SS[.ffffff]`).
- Teto de memória por célula: 1 MiB; acima disso a célula guarda o prefixo e a flag `truncated` (row detail mostra "truncated at 1 MiB"). No grid de tabela o teto também limita a **transferência** (SQL gerado usa `left(col::text, 1048576)` no PG e `LEFT(col, 1048576)` no MySQL). No editor o servidor envia o valor inteiro; o pico de memória durante a recepção é aceito e documentado.

### 4.5 Editor SQL
- Realce por tokens (keywords, strings, números, comentários, identificadores entre aspas, operadores), desligado acima de 1 MiB de texto (com aviso). Números de linha. Fundo sutil no statement sob o cursor.
- **Split**: tokenizer do `sqlparser` no dialeto da conexão (strings, comentários, `$$` do PG). MySQL aceita linhas `DELIMITER xx` (removidas antes de enviar). Erro de tokenização → texto inteiro como 1 statement.
- **Execução**: `Ctrl+Enter` roda a seleção, se houver (todos os statements dentro dela); senão, o statement sob o cursor (ou o anterior mais próximo, se o cursor estiver em espaço após `;`). `Ctrl+Shift+Enter` roda todos. Execução sequencial; para no primeiro erro.
- **Resultado**: abas Result (grid do último statement com linhas; mesmo widget do §4.3, sem sort/filtro) e Messages (por statement: ok/erro, linhas afetadas, duração). `Load more` busca mais `editor_row_limit` linhas do cursor pausado. Rodar de novo descarta o cursor anterior (D27–D29).
- **Erro**: mensagem do servidor; se houver posição (PG `position`, MySQL "at line N"), marca a linha no gutter e sublinha o token.
- **Cancel** (`Ctrl+.` ou botão Stop): PG `CancelToken::cancel_query`; MySQL `KILL QUERY <connection_id>` via conexão efêmera.
- **Autocomplete**: popup na posição do cursor ao digitar identificador (≥ 1 caractere) ou com Ctrl+Space; ↑/↓ navega, Enter/Tab aceita, Esc fecha (teclas consumidas antes do `TextEdit` quando o popup está aberto). Fontes: keywords do dialeto; schemas/tabelas/views do cache da sidebar (carrega sob demanda); colunas das tabelas referenciadas no statement atual, após `alias.` ou dentro de `SELECT`/`WHERE` (alias resolvido por varredura de tokens `FROM`/`JOIN`). Ranking: prefixo antes de substring; máximo 50 itens.
- **Toggle comment** `Ctrl+/` nas linhas selecionadas.
- **Arquivos**: Open (Ctrl+O), Save (Ctrl+S), Save As (Ctrl+Shift+S) via diálogo nativo (`rfd`, portal XDG). Título = nome do arquivo + `•` se sujo. Fechar sujo → Save / Discard / Cancel.
- **Database da aba**: seletor no toolbar. MySQL → `USE`; PG → reconecta a sessão da aba (bloqueado com transação aberta).
- **Histórico** (Ctrl+H, painel lateral): cada statement executado registra `{ts, database, sql, duration_ms, rows, ok, error}`; busca por substring; duplo clique insere no cursor.

### 4.6 Transações
- Begin / Commit / Rollback no toolbar da aba. Estado `Idle | Open | Aborted`, atualizado pelos botões e pela classificação de `BEGIN`/`START TRANSACTION`/`COMMIT`/`ROLLBACK` digitados.
- PG: erro com transação aberta → `Aborted` ("Rollback required"). MySQL: DDL com transação aberta → `Idle` + mensagem "implicit commit".
- Fechar aba, workspace ou app com transação `Open`/`Aborted` → diálogo Commit / Rollback / Cancel.

### 4.7 Política de segurança (`crustdb-core::safety`)
Classificação por statement (AST do `sqlparser`):

| Classe | Exemplos | Veredito |
|---|---|---|
| `Read` | `SELECT`, `WITH` sem DML, `SHOW`, `DESCRIBE`, `EXPLAIN` sem `ANALYZE` (exceto se chamar função da lista de efeito colateral abaixo) | Run |
| `Session` / `Tx` | `SET`, `USE`, `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT` | Run |
| `CreateSafe` | `CREATE` sem `OR REPLACE` | Run |
| `Dml` | `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `REPLACE`, `LOAD DATA`, `COPY … FROM`, `WITH` com DML, `EXPLAIN ANALYZE <dml>` | ver regras |
| `Destructive` | `DROP` (inclusive `DATABASE`/`SCHEMA … CASCADE`), `TRUNCATE`, `ALTER` (inclusive drop de coluna), `RENAME`, `CREATE OR REPLACE`, `GRANT`, `REVOKE` | Confirm (senha só em `DROP TABLE`/`TRUNCATE` acima de X, D24) |
| `Unknown` | não parseável, `CALL`, `DO`, `EXECUTE`, `SELECT` que chama função com efeito colateral conhecido (PG: `pg_terminate_backend`, `pg_cancel_backend`, `pg_reload_conf`, `set_config`, `setval`, `nextval`, `lo_unlink`, `pg_advisory_lock*`, `dblink_exec`; MySQL: `GET_LOCK`, `RELEASE_LOCK`, `RELEASE_ALL_LOCKS`) | Confirm (D22) |

Regras de `Dml`. A estimativa `E` vem de `EXPLAIN` sem `ANALYZE` na sessão do editor; `INSERT … VALUES` usa a contagem de tuplas do AST.
1. `E > threshold` ou `E` desconhecido → **Password**.
2. `UPDATE`/`DELETE` sem `WHERE`, ou `E > 1` → **Confirm**.
3. Senão → **Run**.

- PG: `EXPLAIN (FORMAT JSON)`; `E` = `Plan Rows` do filho do nó `ModifyTable`. MySQL: `EXPLAIN`; `E` = maior `rows` entre as linhas da tabela alvo. Validar ambos em teste de integração.
- Lote (Run all / seleção): avalia todos antes de executar; **um** diálogo lista os statements sinalizados, com classe e estimativa; vale o veredito mais restritivo; Cancel = nada executa. `EXPLAIN` que depende de statement anterior do lote (ex.: tabela ainda não criada) → desconhecido → Password.
- `COPY … FROM`/`LOAD DATA` não têm `EXPLAIN` → sempre Password (fail-safe, intencional). `COPY … FROM STDIN` não é suportado na v1 (erro claro). `LOAD DATA LOCAL INFILE` fica desabilitado no client (nenhum handler de arquivo local registrado).
- **Password**: comparar com o segredo em memória da sessão (comparação em tempo constante); sem senha, ver D25. Três erros fecham o diálogo sem executar.
- Grid, sidebar e export nunca passam pela política: só emitem `SELECT` gerado pelo código, em sessão read-only (`SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` / `SET SESSION TRANSACTION READ ONLY`).

### 4.8 Status bar (ícones `egui-phosphor` + tooltip)
Da esquerda para a direita: estado da conexão (cor) + nome + versão do servidor · cadeado TLS (fechado = verificado, âmbar = `require`/`verify-ca`, cortado = `disable`) · ícone SSH · badge de transação (âmbar Open, vermelho Aborted) · escudo com o threshold (tooltip com as regras do §4.7) · tempo da última query (tooltip: time-to-first-row, fetch total, render) · linhas retornadas/afetadas · latência do ping · CPU% do processo · RSS em MB · exports em andamento (com linhas escritas). Atualização a 1 Hz via `request_repaint_after`.

### 4.9 Atalhos (fixos na v1)

| Ação | Atalho | Ação | Atalho |
|---|---|---|---|
| Nova conexão | Ctrl+N | Run current / selection | Ctrl+Enter |
| Nova aba de query | Ctrl+T | Run all | Ctrl+Shift+Enter |
| Fechar aba | Ctrl+W | Cancel query | Ctrl+. |
| Próxima / anterior aba | Ctrl+Tab / Ctrl+Shift+Tab | Autocomplete | Ctrl+Space |
| Refresh (sidebar/tabela) | F5 | Toggle comment | Ctrl+/ |
| Filtro da tabela | Ctrl+F | Abrir / salvar / salvar como | Ctrl+O / Ctrl+S / Ctrl+Shift+S |
| Próxima / anterior página | Ctrl+] / Ctrl+[ | Histórico | Ctrl+H |
| Row detail | Space (foco no grid) | Export CSV | Ctrl+Shift+E |

### 4.10 Export CSV
- RFC 4180, UTF-8, separador `,`, fim de linha `\n`, cabeçalho com nomes de coluna. `NULL` → campo vazio sem aspas; string vazia → `""`; binário → hex com prefixo `0x`.
- Destino via diálogo nativo; escrita em streaming (`BufWriter`, memória constante); progresso no status bar; cancelável (o arquivo parcial é removido).

### 4.11 Layout (wireframe)

```
┌ cRustDB ──────────────────────────────────────────────────────────────────┐
│ [prod-pg ▾][+] │ users × │ Query 1 • × │ orders × │                        │
│ 🔍 filter      │ [Data|Structure|DDL]                                       │
│ ▾ app_db       │ ┌ column ▾ ┐┌ op ▾ ┐┌ value ┐ [×]  [+] [Apply] [Clear]      │
│  ▾ public      │ ├────┬──────────┬──────────┬───────────┤┌ Row detail ──┐   │
│   ▾ Tables     │ │ id │ email    │ created  │ meta      ││ id      1    │   │
│     users      │ │ 1  │ a@b.c    │ 2026-…   │ {"k":1}   ││ meta {       │   │
│     orders     │ │ 2  │ NULL     │ 2026-…   │ (binary…) ││   "k": 1 }   │   │
│   ▸ Views      │ └────┴──────────┴──────────┴───────────┘└──────────────┘   │
│   ▸ Functions  │ ‹ Prev  Rows 1–300 of ~1.2M  Next ›  [300] ⟳  ⇩ CSV  ▣     │
├────────────────┴───────────────────────────────────────────────────────────┤
│ ● prod-pg PG 16.2 🔒 ⇄SSH TX 🛡1000 ⏱42 ms 300 rows ↯3 ms CPU 1% RSS 84 MB │
└────────────────────────────────────────────────────────────────────────────┘
Aba de query: toolbar [▶ Run][▶▶ Run all][■ Stop][Begin][Commit][Rollback][db ▾][limit 1000]
              editor (split redimensionável) / [Result | Messages] + [Load more] [⇩ CSV]
```

## 5. Components

### Workspace (raiz do repositório)
Repositório: `https://github.com/Desbravando-Rust/crustdb` (privado, branch padrão `main`, já contém `LICENSE` Apache-2.0 e `.gitignore` de Rust). Trabalho em branches de feature e PR; nunca push direto na `main`. Todo `Cargo.toml` declara `license = "Apache-2.0"`; `deny.toml` só aceita licenças compatíveis com Apache-2.0.

| File | Responsibility |
|---|---|
| `Cargo.toml` | `[workspace]` com `members = ["crates/*"]`, `[workspace.dependencies]`, `[workspace.lints]` (`unsafe_code = "forbid"`; `clippy::unwrap_used`/`expect_used` = deny fora de testes), `[profile.release]` (`lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`) |
| `rust-toolchain.toml` | stable mais recente no início (fixar a versão exata); componentes `rustfmt`, `clippy`, `llvm-tools-preview` |
| `deny.toml` | `cargo-deny`: advisories, licenças permitidas, fontes |
| `.github/workflows/ci.yml` | gates do §9 + matriz de bancos |
| `packaging/crustdb.desktop`, `packaging/crustdb.png`, `packaging/build-appimage.sh` | distribuição (D32) |

### `crates/crustdb-core/src/`
| File | Responsibility |
|---|---|
| `lib.rs` | re-exports |
| `model.rs` | `DriverKind`, `ConnectionId` (UUID), `ObjectKind`, `ObjectRef{database, schema: Option, name, kind}`, `TypeCategory`, `ColumnMeta`, `Cell`, `Row`, `ServerVersion`, `Completion{affected_rows: Option<u64>}` |
| `error.rs` | `DriverError{kind: Connect\|Auth\|Tls\|Timeout\|Query{code, position}\|Cancelled\|Unsupported\|Protocol, message}` (`thiserror`) |
| `driver.rs` | traits `Driver`, `Session`, `CancelHandle`, `Introspector`; `ConnectTarget`, `SessionPurpose`, `ExecMode`, `Param`, `ResultHeader`, `Fetched` |
| `dialect.rs` | trait `Dialect` + builder compartilhado `build_table_query` |
| `query.rs` | `TableQuery{object, columns, filters, sorts, limit, offset}`, `Filter`, `Operator`, `operators_for(TypeCategory)`, `Sort` |
| `sql/split.rs` | `split(text, DriverKind) -> Vec<StatementSpan{range: Range<usize>, text}>`, `statement_at(&[StatementSpan], byte_offset)` |
| `sql/classify.rs` | `classify(stmt, DriverKind) -> Classified{class, has_where, values_rows: Option<u64>, target: Option<ObjectRef>}` |
| `sql/highlight.rs` | `highlight(text, DriverKind) -> Vec<(Range<usize>, TokenKind)>` |
| `sql/complete.rs` | `completion_context(text, byte_offset, DriverKind) -> CompletionContext{prefix, expect: Keyword\|Table\|Column{qualifier}, referenced_tables}` |
| `safety.rs` | `SafetyPolicy{threshold}`; `evaluate(&[Assessment]) -> Verdict{Run\|Confirm\|Password, flagged: Vec<Flag>}` |
| `url.rs` | `parse_connection_url(&str) -> Result<ParsedUrl, UrlError>` |
| `format.rs` | `display_cell(&Cell, &ColumnMeta, max_chars) -> Cow<str>`, `pretty_json`, `hex_preview` |

### `crates/crustdb-postgres/src/` e `crates/crustdb-mysql/src/` (mesma forma)
| File | Responsibility |
|---|---|
| `lib.rs` | `PostgresDriver` / `MysqlDriver` implementam `Driver` |
| `connect.rs` | config do client, TLS por modo (D10), hostname real via SSH, timeouts, `SessionPurpose` → read-only |
| `session.rs` | `Session` stateful (D27–D29, D37, D38), `explain_rows`, cancel |
| `dialect.rs` | `Dialect`: aspas (`"` / `` ` ``), placeholders (`$n` / `?`), cast de parâmetro, `contains` |
| `introspect.rs` | `Introspector`: queries de catálogo do §4.2/§4.3, estimativa de linhas, DDL |
| `../tests/integration.rs` | testes com testcontainers (§8) |

### `crates/crustdb-app/src/`
| File | Responsibility |
|---|---|
| `main.rs` | logging (`tracing`, arquivo `$XDG_STATE_HOME/crustdb/crustdb.log`, nível via `CRUSTDB_LOG`), runtime, `eframe::run_native` |
| `app.rs` | `impl eframe::App`: layout raiz, roteamento de atalhos, drenagem de `Event` |
| `runtime.rs` | thread tokio, `Command`/`Event`, `RequestId` |
| `actors/workspace.rs`, `actors/data.rs`, `actors/editor.rs`, `actors/export.rs` | atores (§2) |
| `net/ssh.rs` | túnel `russh`, verificação de `known_hosts` |
| `storage/config.rs` | load/save atômico (temp + rename) de `connections.json`; dirs 0700, arquivos 0600 |
| `storage/secrets.rs` | keyring: service `crustdb`, accounts `<uuid>:db`, `<uuid>:ssh-password`, `<uuid>:ssh-passphrase` |
| `storage/history.rs` | append JSONL, compactação para as últimas 10.000 na abertura, busca |
| `metrics.rs` | amostragem de `/proc/self` (D31) |
| `state/` (`workspace.rs`, `table_tab.rs`, `query_tab.rs`, `tree.rs`, `dialogs.rs`) | estado puro e máquinas de estado (paginação, sort, transação, fluxo de confirmação), testável sem GUI |
| `ui/` (`home.rs`, `connection_form.rs`, `sidebar.rs`, `grid.rs`, `filter_bar.rs`, `structure.rs`, `row_detail.rs`, `editor.rs`, `autocomplete.rs`, `results.rs`, `history_panel.rs`, `status_bar.rs`, `confirm_dialog.rs`, `theme.rs`) | views finas: leem o estado e emitem intenções |

Contratos (forma normativa; nomes podem mudar se a semântica se mantiver):
- `Driver: Send + Sync` → `kind()`, `dialect() -> &dyn Dialect`, `introspector() -> &dyn Introspector`, `async connect(&ConnectTarget, SessionPurpose) -> Result<Box<dyn Session>, DriverError>`.
- `Session: Send` (**stateful**: no máximo 1 resultado ativo, sem borrow entre chamadas, para o ator guardar a sessão e o cursor pausado sem struct autorreferente) → `server_version()`, `cancel_handle() -> Box<dyn CancelHandle>`, `async ping() -> Result<Duration>`, `async start(sql, ExecMode) -> Result<ResultHeader{columns}>`, `async start_params(sql, &[Param]) -> Result<ResultHeader>`, `async fetch(max_rows) -> Result<Fetched{rows, exhausted}>`, `async finish() -> Result<Completion>` (aplica D27–D29 e D37), `has_active_result() -> bool`, `async explain_rows(sql) -> Result<Option<u64>>`, `async use_database(name)` (PG → `Unsupported`), `async close(self: Box<Self>)`. `start` com resultado ativo chama `finish` antes.
- MySQL: como `QueryResult` do `mysql_async` empresta `&mut Conn`, a sessão MySQL é um handle para uma task tokio interna dona da `Conn` (comandos por canal). PG: `Client` + nome do cursor/stream `'static` guardados na sessão.
- Traits com `async-trait`, porque o driver é escolhido em runtime (`Box<dyn …>`).

## 6. Data model (arquivos)

`$XDG_CONFIG_HOME/crustdb/connections.json` (0600):
```json
{ "version": 1, "connections": [ {
  "id": "7f3c…-uuid", "name": "prod-pg", "driver": "postgres",
  "host": "db.example.com", "port": 5432, "user": "app", "database": "app_db",
  "save_password": true,
  "tls": { "mode": "verify-full", "ca_path": null },
  "ssh": { "host": "bastion.example.com", "port": 22, "user": "ops",
           "auth": { "kind": "key", "key_path": "/home/u/.ssh/id_ed25519" } },
  "page_size": 300, "editor_row_limit": 1000, "safety_threshold": 1000 } ] }
```
- `driver`: `postgres` | `mysql`. `tls.mode`: `disable` | `require` | `verify-ca` | `verify-full`. `ssh`: `null` ou objeto; `auth.kind`: `password` | `key` | `agent`.
- `version` desconhecida → não sobrescrever; abrir em modo leitura com erro visível. JSON inválido → backup `connections.json.bak-<ts>` e lista vazia.

`$XDG_DATA_HOME/crustdb/history/<connection-id>.jsonl` (0600), uma linha por statement (`ts` em ISO-8601 UTC):
```json
{"ts":"2026-10-17T14:03:11Z","database":"app_db","sql":"SELECT 1","duration_ms":3,"rows":1,"ok":true,"error":null}
```
`$XDG_CONFIG_HOME/crustdb/known_hosts`: formato OpenSSH. Tamanho e posição da janela: persistência do `eframe`.

## 7. Error handling, security, performance

**Erros**: tipados (`thiserror`) no core e nos drivers; `anyhow` só em `main.rs`. Erro de conexão aparece no formulário ou na aba, sem modal. Erro de query → aba Messages + gutter. Nada de `unwrap`/`expect` fora de testes. Panic em ator → ator reiniciado; a aba mostra "internal error, see log".

**Segurança** (além do §4.7):
- Senhas em `secrecy::SecretString`; `Debug` redigido; nunca em log, histórico ou arquivo.
- SQL do grid e da introspecção: identificadores via `Dialect::quote_ident` (dobra o caractere de aspas); valores sempre bound.
- Logs: SQL só em nível `debug`; parâmetros nunca.
- Limitações conhecidas (documentar no README): a porta local do túnel SSH fica acessível a outros usuários locais durante a sessão; o histórico guarda SQL em claro (pode conter segredos digitados); o CSV não neutraliza fórmulas (CSV injection).

**Orçamentos de performance** (build release; medir e registrar no README):

| Métrica | Meta |
|---|---|
| Início até o primeiro frame | ≤ 300 ms |
| CPU ociosa (1 conexão aberta, sem atividade) | < 1% |
| Overhead do client para decodificar e renderizar 1.000 × 20 células | ≤ 20 ms |
| Primeiras linhas visíveis após a primeira linha do servidor | ≤ 50 ms |
| Frame rolando 100.000 × 20 linhas carregadas | ≤ 16 ms |
| RSS com 100.000 × 20 células de texto (~20 B) | ≤ 200 MB |
| Export CSV em localhost | ≥ 100.000 linhas/s, memória constante (≤ +20 MB) |
| Digitação num arquivo de 5.000 linhas com realce | frame ≤ 16 ms |

Teto conhecido: o grid não virtualiza colunas; tabelas com mais de 200 colunas podem passar de 16 ms (registrar a medição).

## 8. Testing strategy

**Unit — `crustdb-core`** (`#[cfg(test)]` no próprio módulo):
- `split`: `splits_on_semicolons`, `ignores_semicolons_in_strings_and_comments`, `handles_pg_dollar_quotes`, `handles_mysql_delimiter`, `statement_at_cursor_after_semicolon_picks_previous`, `tokenizer_error_returns_whole_text`.
- `classify`: `select_is_read`, `cte_with_delete_is_dml`, `explain_analyze_delete_is_dml`, `update_without_where_flagged`, `insert_values_counts_tuples`, `drop_truncate_alter_are_destructive`, `create_or_replace_is_destructive`, `unparseable_is_unknown`.
- `safety`: `dml_over_threshold_requires_password`, `unknown_estimate_requires_password`, `multi_row_requires_confirm`, `single_row_with_where_runs`, `batch_takes_strictest_verdict`, `destructive_requires_confirm`, `truncate_big_table_requires_password`.
- `query`/`dialect`: SQL e parâmetros golden por dialeto: `pg_contains_escapes_wildcards`, `mysql_in_list_binds_each_value`, `multi_sort_order_preserved`, `default_sort_uses_pk`, `quote_ident_doubles_quote_char`.
- `url`, `format`, `complete`, `highlight`: casos felizes + bordas (unicode, entrada vazia, string não terminada).
- Benchmarks `criterion` (`crates/crustdb-core/benches/`): split, classify, highlight de 5.000 linhas, `display_cell` em 20.000 células.

**Integration — drivers** (`testcontainers`, Docker; tag de imagem via env `CRUSTDB_PG_IMAGE` / `CRUSTDB_MYSQL_IMAGE`). Matriz: `postgres:12` e `postgres:latest`; `mysql:5.6`, `mysql:5.7`, `mysql:8.4`; `mariadb:10.0` (ou a menor tag disponível) e `mariadb:latest`.
- `connects_with_each_tls_mode`, `verify_full_rejects_wrong_hostname`, `metadata_session_rejects_writes`, `lists_tree_levels_per_driver`, `fetches_page_with_filters_and_sorts`, `row_estimate_without_count`, `ddl_for_each_object_kind`, `explain_rows_for_update_delete`, `read_cursor_load_more_bounded_memory` (`generate_series(1, 50_000_000)` pausado, RSS estável), `cancel_long_query`, `write_with_returning_is_drained_not_cancelled`, `transaction_states_and_implicit_commit_mysql`, `pg_aborted_transaction_state`, `export_full_table_streams_csv`, `ssh_tunnel_with_key_password_agent` (container `openssh-server`), `unknown_host_key_rejected`, `verify_full_via_ssh_checks_real_hostname`, `tls_rejects_server_without_tls12`, `paused_cursor_finished_before_next_command`, `idle_paused_cursor_auto_closed`, `pg_editor_columns_have_type_categories`, `grid_caps_cell_transfer_at_1mib`.

**App state — `crustdb-app`** (fake `Driver` em memória em `crates/crustdb-app/tests/support/fake_driver.rs`):
- `pagination_next_enabled_only_when_limit_plus_one`, `stale_request_id_ignored`, `sort_header_cycle_and_shift_multi`, `close_tab_with_open_tx_prompts`, `confirm_flow_cancel_executes_nothing`, `password_flow_three_failures_aborts`, `config_roundtrip_has_no_secrets`, `config_written_atomically_with_0600`, `history_compacts_to_limit`.
- UI smoke com `egui_kittest`: `connect_and_open_table_renders_grid`, `run_query_shows_result_and_messages`.

**Acceptance** (cada uma vira teste ou checklist manual com evidência):

| AC | Critério |
|---|---|
| AC-01 | Salvar conexão PG e MySQL, reabrir o app e conectar sem redigitar a senha (keyring presente) |
| AC-02 | Sem Secret Service: senha pedida a cada conexão; nenhum segredo em disco (grep em `$XDG_*`) |
| AC-03 | Conectar via SSH (senha, chave e agent) com `verify-full` |
| AC-04 | Importar URL preenche o formulário corretamente |
| AC-05 | Sidebar lista todos os níveis e pastas do §4.2 nos dois bancos |
| AC-06 | Abrir tabela de 1M linhas: primeira página sem `COUNT(*)` (verificado no log do servidor) |
| AC-07 | Filtro `=` em inteiro e `contains` em texto retornam o esperado; aspas e `%` no valor não quebram nem injetam |
| AC-08 | Sort múltiplo estável entre páginas |
| AC-09 | Structure, DDL e row detail corretos para cada tipo de objeto |
| AC-10 | `Ctrl+Enter` roda só o statement sob o cursor; `Ctrl+Shift+Enter` roda todos; o grid mostra o último com linhas |
| AC-11 | `DELETE FROM t` (sem `WHERE`) pede confirmação; `UPDATE` estimado acima de 1.000 linhas pede senha; Cancel não executa nada |
| AC-12 | Cancelar `SELECT pg_sleep(60)` / `SELECT SLEEP(60)` em ≤ 1 s |
| AC-13 | Begin → DML → Rollback desfaz; fechar aba com transação aberta pergunta |
| AC-14 | Export CSV da tabela filtrada e do resultado completo da query, sem re-executar (verificado no log do servidor) |
| AC-15 | Status bar mostra todos os indicadores do §4.8, com tooltips |
| AC-16 | Orçamentos do §7 medidos e dentro da meta |

## 9. Quality gates (CI e antes de cada commit)
`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · integração com Docker (matriz completa no CI) · `cargo llvm-cov` com piso de linhas: core ≥ 90%, drivers ≥ 70%, `crustdb-app/src/state` ≥ 70% (`ui/` excluído) · `cargo deny check` · `cargo build --release`.

## 10. Out of scope v1 (backlog v2)
Edição de dados · cópia para clipboard · atalhos customizáveis · tema claro · outros bancos · SQLite · import · backup/restore · gestão de usuários · process list · diagrama ER · IA · beautify · find/replace no editor · paginação keyset · export além de CSV · grupos/cores/tags de conexão · certificado de cliente TLS (mTLS) · várias abas de resultado · sort/filtro no resultado do editor · export de URL · Windows/macOS.

## 11. Left to the builder
Nomes internos de tipos e módulos privados; layout fino e espaçamentos; cores exatas do tema escuro; lista de keywords por dialeto; algoritmo de backoff além do especificado; formato do log; script de AppImage (`linuxdeploy` ou equivalente) e dependências de runtime declaradas.
