# Labels de conexão persistidas em connections.json (v1 compatível)

* **Status:** Invalidado (substituído pelo ADR 0007, #20)
* **Data:** 2026-10-29
* **Issue Relacionada:** #128

## Contexto e Problema
A issue #128 pede labels (nome + cor) editáveis pelo usuário, com 4 padrão (`local`, `dev`, `staging`, `production`), várias por conexão e filtro na lista de conexões. Isso exige persistir um catálogo global de labels e a associação por conexão, mudando o schema de `connections.json` (D8). Configs já salvas (version 1, sem labels) precisam continuar abrindo sem cair em `backup_and_reset`.

## Opções Consideradas
1. Campo `labels` no topo de `connections.json` (catálogo `[{name, color}]`) + `labels: Vec<String>` (nomes) em `ConnectionConfig`, ambos `#[serde(default)]`, mantendo `version: 1`.
2. Arquivo separado `labels.json` com seu próprio load/save atômico.
3. Bump para `version: 2` com migração explícita.
4. Labels inline por conexão (nome + cor repetidos em cada conexão), sem catálogo.

## Decisão Escolhida
Escolhemos a **Opção 1** porque é aditiva: `serde(default)` faz arquivos v1 antigos carregarem com catálogo padrão (as 4 labels) e conexões sem labels; o save atômico 0600 existente cobre o catálogo sem novo arquivo. `ConnectionConfig.labels` usa `skip_serializing_if = "Vec::is_empty"` para manter o roundtrip do JSON da spec idêntico. Referência por nome (único, validado) evita IDs; renomear/excluir uma label atualiza todas as conexões numa função pura em `storage/config.rs`. Cor persistida como `[u8; 3]` (RGB).

### Consequências Positivas
* Sem bump de versão nem migração; arquivos antigos abrem normalmente.
* Um único arquivo e um único caminho de escrita atômica.
* Binários antigos ignoram os campos novos (serde não usa `deny_unknown_fields`).

### Consequências Negativas / Trade-offs
* Binário antigo que salvar o arquivo descarta labels silenciosamente (aceitável antes de v0.1.0).
* Catálogo ausente vs. catálogo esvaziado pelo usuário: o default só se aplica quando o campo `labels` não existe; lista vazia salva continua vazia.
* Rename/delete precisam propagar para as conexões (por nome), em vez de chave estável.
* Binário antigo (pré-#128) que regravar o arquivo o salva sem `labels`: um downgrade perde o catálogo e as atribuições por conexão.
