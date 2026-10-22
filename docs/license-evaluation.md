# Avaliação de licença do CrustDB (BSL 1.1 e alternativas)

> Aviso: este documento é um insumo técnico, não um parecer jurídico. A decisão final deve passar por um advogado. Os textos oficiais prevalecem sobre qualquer resumo aqui.

Issue: #43. Status: proposta, nenhuma licença foi alterada.

## 1. Objetivos

1. O código-fonte é público e aberto para leitura, uso e modificação.
2. Ninguém pode vender o CrustDB (ou um serviço cujo valor principal seja ele) diretamente.
3. Uso livre por qualquer pessoa ou empresa, inclusive comercialmente, desde que o CrustDB não seja a atividade-fim do negócio (por exemplo, uma equipe usar o CrustDB para administrar seus bancos é permitido; empacotar e revender o CrustDB não é).

## 2. Situação atual

- Licença: Apache-2.0 (`LICENSE`, `license = "Apache-2.0"` no `Cargo.toml` raiz; os crates herdam via workspace). O `README.md` mostra um badge (linha 12) e uma seção de licença (linha 159).
- A Apache-2.0 permite uso comercial, modificação, redistribuição e revenda sem pagamento nem permissão. Não atende ao objetivo 2.
- Autor único no histórico git (zejuniortdr). Isso facilita uma troca de licença, pois não há contribuições de terceiros a relicenciar. Isso deixa de valer assim que houver PRs externos aceitos.
- Só `crustdb-testkit` tem `publish = false`; os demais crates são publicáveis.
- Confusão a evitar: as exceções `BSL-1.0` no `deny.toml` (`clipboard-win`, `error-code`) são a Boost Software License 1.0, permissiva e sem relação com a Business Source License 1.1.

## 3. Business Source License 1.1 (BSL 1.1)

Texto oficial: https://mariadb.com/bsl11/

Estrutura (parâmetros preenchidos pelo licenciante):

- Licensed Work: o CrustDB, na versão indicada.
- Additional Use Grant: uso de produção permitido além do que a BSL base permite. A BSL base proíbe uso em produção; o grant é o que libera o uso desejado.
- Change Date: data em que cada versão passa à Change License. Máximo de 4 anos após a publicação daquela versão.
- Change License: licença para a qual a versão converte. Precisa ser GPL 2.0 ou posterior, ou uma licença compatível com ela. A Apache-2.0 se qualifica por ser compatível com a GPLv3.

Rascunho de Additional Use Grant (a ser revisado por advogado, não é texto oficial):

> You may make production use of the Licensed Work, provided you do not use it to offer a product or service to third parties whose value derives entirely or substantially from the functionality of the Licensed Work (for example, selling, sublicensing for a fee, or hosting it as a database client or administration service).

Parâmetros sugeridos: Change Date = 4 anos após cada release; Change License = Apache-2.0.

O que muda para quem usa:

- Pode baixar, compilar, modificar e usar em produção dentro do grant, inclusive em empresas.
- Não pode revender nem oferecer como serviço pago nos termos excluídos pelo grant.
- Após o Change Date de cada versão, aquela versão vira Apache-2.0 e a restrição some para ela.

## 4. Alternativas

Textos oficiais: fsl.software, polyformproject.org, elastic.co/licensing/elastic-license, gnu.org/licenses/agpl-3.0.

| Opção | Impede revenda/concorrência direta | Uso livre por empresas (não atividade-fim) | Converte para open source | Observação |
|---|---|---|---|---|
| Apache-2.0 (atual) | Não | Sim | Já é open source | Não atende ao objetivo 2 |
| BSL 1.1 + AUG + Apache-2.0 | Sim, conforme o grant | Sim, conforme o grant | Sim, até 4 anos | Grant é redigido por nós; ambiguidade é nossa |
| FSL-1.1-ALv2 | Sim (proíbe uso concorrente) | Sim (qualquer uso permitido exceto concorrente) | Sim, 2 anos após cada release | Texto padronizado, sem grant a redigir |
| Elastic License 2.0 | Não impede a revenda do binário; limita serviço gerenciado, contornar license keys e remover avisos | Sim | Não converte | Não atende ao objetivo 2 |
| PolyForm Noncommercial | Sim, mas proíbe todo uso comercial | Não (bloqueia empresas) | Não | Fere o objetivo 3 |
| PolyForm Shield | Sim (proíbe competir com o produto) | Sim | Não | Parecida com FSL, sem conversão |
| PolyForm Small Business | Limita por porte da empresa | Só pequenas empresas sem licença paga | Não | Fere o objetivo 3 para empresas maiores |
| AGPL-3.0 | Não: permite venda, desde que o código seja aberto | Sim | Já é open source (OSI) | Copyleft de rede; não impede revenda |
| Apache-2.0 + Commons Clause | Sim (proíbe "Sell") | Sim | Não | Definição de "Sell" é vaga; não é open source |
| Dual license (AGPL + comercial) | Parcial: quem quiser fugir do copyleft compra licença | Sim, sob AGPL | Já é open source (OSI) | Exige titularidade total do código (CLA) |

Leitura contra os objetivos: FSL e BSL atendem 1, 2 e 3 e convertem para Apache-2.0. As demais ou bloqueiam empresas, ou não convertem, ou não impedem a venda.

## 5. Riscos e tradeoffs

- Rótulo open source: a OSI não reconhece BSL, FSL, Elastic, PolyForm nem Commons Clause como open source. O termo correto é "source-available". O README e o site não devem chamar o projeto de open source depois da troca.
- Não retroativo: versões já publicadas sob Apache-2.0 continuam Apache-2.0 para quem as obteve. A nova licença vale apenas para releases posteriores.
- Contribuições externas: para manter o direito de relicenciar ou fazer dual license, é preciso CLA ou cessão de copyright antes de aceitar PRs. O DCO não cede direitos de relicenciamento; só basta em inbound=outbound sem relicenciamento futuro. Isso reduz a participação da comunidade.
- crates.io: licenças fora da lista SPDX aceita como expressão exigem `license-file` em vez de `license`. BSL-1.1 e FSL têm identificadores SPDX (`BUSL-1.1`, `FSL-1.1-ALv2`; há também a variante `FSL-1.1-MIT`); confirmar no momento da troca qual campo usar.
- Dependentes: projetos que dependem do CrustDB via `cargo-deny` precisam de exceção na sua `allow`. Isso só se aplica a bibliotecas; o app é um binário final.
- `deny.toml` do projeto: precisa mudar. O `cargo deny check` também verifica a licença dos crates do workspace, e o arquivo não tem `[licenses.private] ignore = true`. Sem ajuste, a troca quebra o gate: é preciso adicionar `[licenses.private] ignore = true` ou a nova licença em `allow`.
- Distribuições Linux: Debian, Fedora e similares não empacotam software fora de suas políticas de licença livre, o que limita a distribuição por repositórios oficiais.
- Adoção: algumas empresas barram licenças source-available por política interna; espere perda de usuários e contribuidores.
- Atividade-fim: o limite entre "usar" e "ter como atividade-fim" é subjetivo e caro de fazer valer na prática. Um grant mais estreito (FSL: "uso concorrente") reduz a ambiguidade.
- Autor único: hoje a troca depende só do titular. Depois de contribuições externas sem CLA, depende de todos os contribuidores.

## 6. Funding

Nenhum impedimento. O arquivo `.github/funding.yml` (`github: Desbravando-Rust`, `buy_me_a_coffee: desbravandorust`) continua válido com qualquer opção; doações não dependem da licença do código. Nada a alterar nele.

## 7. Recomendação

FSL-1.1-ALv2.

Justificativa: atende aos três objetivos com texto padronizado e revisado por terceiros, sem precisar redigir um Additional Use Grant próprio (a parte mais frágil da BSL). Converte para Apache-2.0 em 2 anos por release. A BSL 1.1 com AUG e Change License Apache-2.0 é equivalente em efeito e preferível só se for necessário um prazo de conversão de até 4 anos ou um grant sob medida.

Ressalva: nenhuma das duas é open source segundo a OSI.

Se o rótulo OSI for inegociável: AGPL-3.0 com dual license comercial. Nesse caso o objetivo 2 não é plenamente atendido (a AGPL permite venda com código aberto) e o CLA é indispensável.

## 8. Próximos passos (nada disso foi executado)

- [ ] Aprovação da opção pelo mantenedor.
- [ ] Revisão jurídica do texto escolhido e, se BSL, do Additional Use Grant.
- [ ] ADR em `docs/adr/0001-licenca-<escolha>.md`.
- [ ] Substituir `LICENSE` e registrar a data/versão de corte (releases anteriores seguem Apache-2.0).
- [ ] Ajustar `Cargo.toml` (`license` ou `license-file`) conforme a decisão sobre o crates.io.
- [ ] Atualizar `README.md` (badge e seção de licença; remover "open source" se for source-available).
- [ ] Definir CLA ou cessão de copyright (DCO não basta para relicenciar) antes de aceitar contribuições externas.
- [ ] Ajustar `deny.toml`: `[licenses.private] ignore = true` ou a nova licença em `allow`, para o gate `cargo deny check` continuar passando.
- [ ] Entrada no `CHANGELOG.md`.
