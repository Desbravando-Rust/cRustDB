# Paginação keyset na aba de tabela

* **Status:** Aceito
* **Data:** 2026-10-31
* **Issue Relacionada:** #18

## Contexto e Problema
A aba de tabela pagina com `LIMIT n+1 OFFSET p*n` (spec D15, `build_table_query` em `crustdb-core/src/dialect.rs`). Com OFFSET profundo, o banco lê e descarta todas as linhas anteriores. A issue #18 pede paginação keyset quando a ordenação permite, sem perder as garantias atuais: valores sempre como parâmetros, identificadores via `Dialect::quote_ident`, nenhum IO em `crustdb-core` e nenhuma espera na thread do `egui`.

## Opções Consideradas
1. Nova função `build_keyset_query` e novo `Command` para "próxima/anterior página", com página anterior por ordem invertida e linhas reinvertidas (proposta da issue).
2. Campo novo `TableQuery::after: Option<Vec<String>>` interpretado por `build_table_query`. A aba guarda a âncora de cada página visitada, e a página anterior reusa a âncora guardada em vez de inverter a consulta.

## Decisão Escolhida
Escolhemos a **Opção 2**. O `TableQuery` já viaja de `TableTab::query` por `Command::FetchPage` até o `DataActor`, então nada muda no protocolo, no `DataActor` nem nos drivers. Guardar as âncoras evita um segundo caminho de SQL (ordem invertida e reinversão de linhas) e mantém o recarregamento da página atual depois de `UpdateCell`/insert/delete (ADR 0010) idêntico: a mesma âncora gera a mesma página.

Regras:
* `after` são os valores das colunas de `sorts` (na ordem de `sorts`) da última linha da página anterior. Com `Some`, `build_table_query` adiciona o predicado keyset aos filtros com `AND` e emite `LIMIT` sem `OFFSET`. O tamanho de `after` diferente do de `sorts` é erro (`QueryError`).
* Predicado: com todas as direções iguais, comparação de tupla `(k1, k2) > (p1, p2)` (`<` se todas `DESC`), que os três dialetos aceitam e que usa índice. Com direções mistas, expansão em `OR`: `k1 > p1 OR (k1 = p1 AND k2 < p2) ...`. A expressão de coluna é a mesma do `ORDER BY` (`tabela.coluna` qualificada), e cada valor passa por `Dialect::param` (no PG, `CAST($n AS tipo)`).
* `keyset::after(q, row) -> Option<Vec<String>>` (puro, em `crustdb-core`) decide se a página seguinte pode usar keyset. Retorna `None`, e a aba continua com OFFSET, quando:
  * `sorts` não cobre todas as colunas de PK (view, tabela sem PK);
  * alguma coluna de `sorts` é anulável (`nullable != Some(false)`) e não é PK. `NULL` ordena em lugares diferentes por dialeto e `col > p` nunca casa `NULL`, então até uma âncora não nula perderia linhas;
  * alguma coluna de `sorts` é `Float` (o texto exibido pode não ser o valor guardado), `Json`, `Binary` ou `Other`;
  * a célula da âncora é `Null` ou opaca (`edit::is_opaque`: binária, truncada ou no limite de `CELL_MEMORY_CAP`).
* `TableTab::query` acrescenta as colunas de PK que faltam em `sorts` como desempate `ASC`. Isso torna a ordenação determinística também no OFFSET e não muda o cabeçalho da grade, que lê `TableTab::sorts`.
* A aba guarda `anchors[p]` (âncora da página `p`, `anchors[0] = None`). `apply_page` calcula a âncora da página seguinte a partir da última linha devolvida pelo servidor, antes de qualquer linha provisória de insert. Mudança de ordenação ou filtro volta à página 0 e limpa as âncoras.
* Pular direto para a página N continua com OFFSET. Hoje a UI não tem esse controle, então nada a fazer.

### Consequências Positivas
* Página profunda deixa de ler e descartar as linhas anteriores quando há índice compatível com a ordenação (a PK, no caso padrão).
* Nenhuma mudança em protocolo, `DataActor` ou drivers. A exportação continua igual (`after: None`, sem `LIMIT`).
* O desempate por PK deixa o OFFSET estável quando a coluna ordenada tem valores repetidos.

### Consequências Negativas / Trade-offs
* `TableQuery` (API pública de `crustdb-core`) ganha um campo, e todo literal de `TableQuery` precisa de `after: None`.
* A página anterior reflete a âncora antiga: se outra sessão inserir linhas antes dela, a página pode ter outro conteúdo que o de antes. Também acontece com OFFSET.
* Ordenação por coluna anulável, `Float` ou opaca continua lenta em página profunda.
* MySQL compara coluna inteira ou decimal com parâmetro texto como `double`. Chaves acima de 2^53 ou decimais com muitos dígitos podem repetir uma linha na borda da página. Os filtros e o `UPDATE` por PK já têm o mesmo limite.
* Chave única não nula como desempate (sem PK) fica de fora até alguém pedir.
