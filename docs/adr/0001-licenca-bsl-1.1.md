# Licença do projeto: Business Source License 1.1

* **Status:** Aceito
* **Data:** 2026-10-22
* **Issue Relacionada:** #43

## Contexto e Problema
O cRustDB era Apache-2.0, que permite a qualquer um revender o produto. O objetivo é manter o código público e o uso livre, inclusive em empresas, mas impedir que terceiros vendam o cRustDB ou um produto/serviço baseado nele. A avaliação completa está em `docs/license-evaluation.md`. Este registro não é parecer jurídico.

## Opções Consideradas
1. Manter Apache-2.0.
2. Business Source License 1.1 com Additional Use Grant.
3. Functional Source License (FSL-1.1-ALv2).
4. AGPL-3.0 + licença comercial (dual license).

## Decisão Escolhida
Escolhemos a **Opção 2** (`BUSL-1.1`), com estes parâmetros:

* **Licensor:** Desbravando-Rust.
* **Additional Use Grant:** uso de produção livre, inclusive em empresas, exceto vender, revender, sublicenciar ou oferecer mediante pagamento o cRustDB ou produto/serviço cujo valor venha substancialmente dele.
* **Change Date:** quatro anos após a publicação de cada versão.
* **Change License:** Apache-2.0.

A BSL permite redigir o grant exatamente para o objetivo (vetar revenda, liberar uso interno) e converge para Apache-2.0. A FSL tem prazo fixo de 2 anos e grant padronizado. A AGPL não impede a venda.

### Consequências Positivas
* Revenda direta e serviços pagos baseados no cRustDB passam a exigir licença comercial.
* Uso interno em empresas continua livre.
* Cada versão vira Apache-2.0 após 4 anos.

### Consequências Negativas / Trade-offs
* Pela definição da OSI, deixa de ser open source. Fica source-available até a Change Date.
* Versões já publicadas sob Apache-2.0 continuam Apache-2.0, porque a troca não é retroativa.
* Contribuições externas passam a exigir CLA para preservar a possibilidade de relicenciar.
* `deny.toml` precisa permitir `BUSL-1.1`, porque o cargo-deny verifica os crates do workspace.
* Distros Linux não empacotam BSL no repositório principal.
* "Valor substancialmente derivado" é um critério interpretativo. A revisão jurídica deve validar o texto do grant.
