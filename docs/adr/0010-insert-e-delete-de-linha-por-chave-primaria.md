# INSERT e DELETE de linha na aba de tabela

* **Status:** Aceito
* **Data:** 2026-10-31
* **Issue Relacionada:** #5

## Contexto e Problema
As ADRs 0003 e 0004 cobrem só o `UPDATE` por PK, com alterações pendentes de uma linha e um único statement no "Save". A issue #5 pede também inserir e excluir linhas na aba de tabela, com as mesmas garantias: sessão `Data` somente leitura, valores sempre como parâmetros, identificadores via `Dialect::quote_ident` e nenhuma espera na thread do `egui`.

## Opções Consideradas
1. Novo `Command`/`Event` por operação (`InsertRow`, `DeleteRows`) e um `DELETE` por linha, numa transação.
2. Duas funções puras novas em `crustdb-core` (`edit::build_insert` e `edit::build_delete`). O `BuiltQuery` resultante segue pelo `Command::UpdateCell` existente, que já recebe um statement pronto e roda na sessão `Editor` da aba. A exclusão de N linhas vira um único `DELETE`.

## Decisão Escolhida
Escolhemos a **Opção 2**. O caminho `UpdateCell` -> `DataActor::update_cell` -> `CellUpdated` já faz o que as duas operações precisam: abre a sessão `Editor` sob demanda, roda um statement em autocommit, avisa "row not found" quando `affected_rows == Some(0)` e recarrega a página quando dá certo. Um único statement já é atômico, então não precisa de transação, e nada muda no `DataActor`, no protocolo nem nos drivers. Só os comentários de `UpdateCell` e `CellUpdated` passam a dizer "statement de escrita da aba".

Regras:
* Mesmo gate das ADRs 0003 e 0004: só `ObjectKind::Table` com PK. Em view ou tabela sem PK, "Add row" e "Delete rows" ficam desabilitados, com tooltip explicando o motivo.
* `build_insert(d, object, columns, values: &[(usize, Option<String>)])` gera `INSERT INTO t (a, b) VALUES (p1, p2)` só com as colunas preenchidas. As colunas omitidas ficam com o default do banco. `None` grava `NULL` explícito. Uma lista vazia é rejeitada: `DEFAULT VALUES` muda de sintaxe entre dialetos e fica de fora.
* `build_delete(d, object, columns, rows: &[&[Cell]])` gera `DELETE FROM t WHERE (pk1 = p AND pk2 = p) OR (...)` com todas as colunas de PK de cada linha. Ele usa o mesmo construtor de `WHERE` do `build_update`, então uma PK `Null`, binária ou truncada é rejeitada da mesma forma.
* "Add row" acrescenta uma linha provisória (só `Cell::Null`) à página, que vira a `pending_row`. As células são editadas pelo fluxo de pendências da ADR 0004, e "Save" envia o `INSERT`. Só pode haver uma linha pendente por vez, seja inserção ou edição.
* "Delete rows" age sobre a seleção da grade (cursor e âncora) e pede confirmação com a quantidade de linhas. Fica bloqueado enquanto houver alteração pendente.
* Quando dá certo, a página recarrega, como no `UPDATE`.

### Consequências Positivas
* Nenhuma mudança em drivers, `SessionPurpose`, `DataActor` ou protocolo. Na API pública de `crustdb-core` entram só as duas funções.
* Um statement e uma recarga por operação, inclusive na exclusão de várias linhas.

### Consequências Negativas / Trade-offs
* O nome `UpdateCell` passa a carregar `INSERT` e `DELETE`. Renomear fica para depois, se o nome atrapalhar a leitura.
* Se o `DELETE` apagar só parte das linhas (`affected_rows` menor que a seleção), o caso não é avisado. Só `Some(0)` gera "row not found".
* O tamanho do `DELETE` acompanha o número de linhas selecionadas multiplicado pelas colunas de PK, e o limite fica no tamanho da página. O menor teto de parâmetros entre os dialetos é o do SQLite (32766).
* Na linha provisória não dá para distinguir "default do banco" de `NULL` digitado antes de salvar: as duas aparecem como `NULL` até a recarga.
