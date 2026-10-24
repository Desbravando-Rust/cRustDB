# Edição de célula por chave primária em sessão gravável separada

* **Status:** Aceito
* **Data:** 2026-10-23
* **Issue Relacionada:** #5

## Contexto e Problema
A v1 é somente leitura fora do editor SQL (spec §10): toda sessão com `SessionPurpose` diferente de `Editor` roda `READ ONLY` (Postgres/MySQL) ou abre o arquivo com `SQLITE_OPEN_READ_ONLY` (SQLite). A issue #5 pede edição de dados na aba de tabela. Para isso é preciso:

* gerar `UPDATE` seguro (valores sempre como parâmetros, identificadores via `Dialect::quote_ident`) a partir das colunas do catálogo, que já trazem tipo real e flag de PK;
* gravar sem quebrar a garantia de que leitura de páginas, estrutura e export nunca escrevem;
* não bloquear a thread do `egui`.

## Opções Consideradas
1. Tornar a sessão `Data` gravável.
2. Novo `SessionPurpose::Write` em `crustdb-core`.
3. O `DataActor` abre sob demanda uma segunda sessão `SessionPurpose::Editor` só para os `UPDATE`s; o SQL vem de uma função pura nova em `crustdb-core` (`edit::build_update`).

## Decisão Escolhida
Escolhemos a **Opção 3**. A sessão `Data` continua somente leitura, então um bug na paginação não tem como escrever. `Editor` já é o único propósito gravável nos três drivers, logo nenhum driver muda e o enum da API pública fica igual. O `UPDATE` roda no task tokio do `DataActor` (mesma mailbox de `FetchPage`), sem tocar a UI thread. A geração do SQL fica em `crustdb-core`, que não faz IO, e pode ser testada sem banco.

Regras do escopo mínimo:
* Só `ObjectKind::Table` que tenha PK. O `WHERE` usa todas as colunas de PK com `Dialect::param`, que no Postgres faz `CAST` para o tipo do catálogo.
* Não editável: célula alvo `Bytes` ou truncada, e linha cuja PK tenha `Null`, `Bytes` ou valor truncado.
* Autocommit, um `UPDATE` por edição. `affected_rows == Some(0)` gera um aviso ("row not found"). Com sucesso, a página é recarregada.
* Valor `None` grava `NULL`.

### Consequências Positivas
* A garantia "só o editor escreve" vale por sessão. Não existe sessão de dados gravável.
* Nenhuma mudança nos drivers nem em `SessionPurpose`. Na API pública entra só `edit::build_update` e variantes novas de `QueryError`.

### Consequências Negativas / Trade-offs
* Cada aba que edita abre uma conexão a mais, que vive enquanto a aba estiver aberta.
* Sem transação, sem lote de alterações pendentes e sem desfazer. Cada Enter vai direto para o banco.
* Ficam de fora: INSERT/DELETE de linha, edição em resultado do editor SQL, tabelas sem PK, colunas binárias.
