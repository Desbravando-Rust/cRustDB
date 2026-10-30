# Label única por conexão, pastas na home e busca (substitui ADR 0006)

* **Status:** Aceito
* **Data:** 2026-10-30
* **Issue Relacionada:** #20

## Contexto e Problema
A issue #20 pede organizar conexões salvas: seções expansíveis na home, faixa de cor na lista e na barra do workspace aberto e busca por texto. O ADR 0006 (#128) já persiste um catálogo de labels (nome + cor) em `connections.json` e permite **várias** labels por conexão (`ConnectionConfig.labels: Vec<String>`). Pastas e faixa de cor exigem uma única label por conexão: com várias, a conexão apareceria em mais de uma pasta e a cor da faixa ficaria ambígua. Criar um campo `group` separado duplicaria o conceito de label.

## Opções Consideradas
1. Trocar `labels: Vec<String>` por `label: Option<String>` (no máximo uma label, que serve de pasta e cor). Leitura aceita o `labels` antigo, que fica com a primeira label. Escrita grava `label`. `version` continua 1.
2. Manter `labels: Vec<String>` e acrescentar `group: Option<String>` e `color` (proposta original da issue).
3. Opção 1 com bump para `version: 2` e migração explícita.

## Decisão Escolhida
Escolhemos a **Opção 1** porque um único conceito (label) cobre pasta, cor e filtro sem campo novo, e a mudança é retrocompatível na leitura, então dispensa bump de versão (§6).

* **Schema:** `#[serde(default, alias = "labels", deserialize_with = ..., skip_serializing_if = "Option::is_none")] pub label: Option<String>`. O deserializador é leniente: aceita `null`, string ou array (fica com o primeiro item string). Qualquer outro tipo vira `None`. Um campo legado malformado não deve derrubar `connections` inteiro em `backup_and_reset`.
* **Migração:** implícita na leitura. Config antiga com várias labels fica com a **primeira** e o restante se perde no próximo save. Conexão sem label continua sem o campo no JSON, e o roundtrip do JSON da spec continua idêntico.
* **Catálogo:** `labels: [{name, color}]` no topo do arquivo não muda. Rename atualiza `label` das conexões. Delete deixa `label = None`.
* **Home:** funções puras em `state/home_filter.rs`. `filter_connections` faz busca case-insensitive por nome, host e label, em AND com o filtro de label existente. `group_by_label` monta uma pasta por label, na ordem do catálogo, com "Ungrouped" no fim. Uma label que não está no catálogo vira pasta própria com cor de fallback.
* **Downgrade:** não é suportado. Um binário anterior a esta mudança ignora `label` (não há `deny_unknown_fields`). A conexão abre sem label e um save remove a atribuição. O catálogo é preservado. Gravar `labels` também para compatibilidade foi rejeitado: com o `alias`, os dois campos juntos causam erro de campo duplicado, e o problema só afeta builds anteriores à v0.1.0.

### Consequências Positivas
* Não há bump de versão nem migração explícita. Arquivos v1, com ou sem `labels`, continuam abrindo.
* Pasta, cor da faixa e filtro saem da mesma fonte, e cada conexão aparece em exatamente uma pasta.
* `rename_label`/`delete_label` continuam funções puras em `storage/config.rs`.

### Consequências Negativas / Trade-offs
* Configs com várias labels perdem todas menos a primeira, sem aviso, no primeiro save.
* Um arquivo editado à mão com `label` e `labels` juntos falha a desserialização por campo duplicado e cai em `backup_and_reset`.
* Downgrade perde as atribuições por conexão (veja acima).
* O deserializador customizado precisa de testes próprios para string, array, array vazio, `null` e tipo inválido.
