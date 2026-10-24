# Alterações pendentes por linha e um único UPDATE confirmado

* **Status:** Aceito
* **Data:** 2026-10-24
* **Issue Relacionada:** #84

## Contexto e Problema
A ADR 0003 definiu "autocommit, um `UPDATE` por edição": cada Enter (ou "Set NULL") na grade da aba de tabela vai direto para o banco. Editar N colunas da mesma linha gera N `UPDATE`s e N recargas de página, e uma edição acidental é gravada sem chance de revisão. A issue #84 pede que as alterações de uma linha virem um único `UPDATE` pela PK e que o usuário confirme antes de gravar.

## Opções Consideradas
1. Manter `edit::build_update` de uma coluna e enviar vários `UPDATE`s numa transação ao salvar.
2. Acumular alterações pendentes no estado da aba (`crustdb-app`) e trocar a assinatura de `edit::build_update` em `crustdb-core` para receber a lista de `(coluna, valor)` de uma linha, gerando `UPDATE t SET a = $1, b = $2 WHERE <pk>`.

## Decisão Escolhida
Escolhemos a **Opção 2**. Um único statement já é atômico, então não precisa de transação nem de mudança no `DataActor`, no protocolo (`Command::UpdateCell` já leva um `BuiltQuery`) ou nos drivers. A geração do SQL continua pura em `crustdb-core` e testável sem banco.

Regras:
* Enter / "Set NULL" passam a só registrar a alteração pendente (sem `Command`). A célula pendente aparece destacada com o valor novo.
* As pendências valem para uma linha por vez. "Save" envia um `UPDATE` com todas as colunas alteradas da linha, e "Discard" descarta todas.
* O `WHERE` usa os valores de PK originais da página, inclusive quando a própria PK foi editada.
* Valor igual ao original não entra na lista. Lista vazia não gera `UPDATE`.
* As demais regras da ADR 0003 continuam valendo: sessão `Editor` separada, células opacas não editáveis, aviso com `affected_rows == Some(0)`, recarga da página quando dá certo.

### Consequências Positivas
* Um `UPDATE` e uma recarga por linha, em vez de um por célula.
* Nada é gravado sem uma ação explícita de "Save".

### Consequências Negativas / Trade-offs
* Muda a API pública de `crustdb-core` (`edit::build_update`), e os testes de integração dos drivers que a chamam precisam ser atualizados.
* Recarregar, paginar, ordenar ou filtrar com alterações pendentes descarta essas alterações ou é bloqueado (ver issue #84). Edição em lote de várias linhas fica de fora.
