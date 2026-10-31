# IA local para escrita de query (spike)

* **Status:** Proposto
* **Data:** 2026-10-29
* **Issue Relacionada:** #125

## Contexto e Problema
A issue #125 pede um spike: quanto custa introduzir IA na escrita de query usando um modelo local. A spec v1 lista IA como fora do escopo (§10). Este ADR registra a avaliação e a recomendação. Ele é uma decisão de avaliação do spike e não autoriza implementação, que não está agendada. Aceitar este ADR é pré-requisito para abrir as issues de implementação (uma por passo do plano abaixo). Até lá nada na §10 da spec é revogado.

Esforço e latência citados neste ADR são estimativas. A medição fica para a issue do PoC e é condição para aceitar o ADR. 
Requisitos que qualquer solução precisa respeitar:

* **Local e privado.** Nomes de schema, tabela e coluna, e o texto da query, não saem da máquina por padrão. Nada de API em nuvem como caminho padrão.
* **UI nunca bloqueia.** Inferência leva segundos. Ela roda fora da thread do `egui`, no mesmo padrão de actors do `crustdb-app` (`actors/editor`, `actors/workspace`).
* **`crustdb-core` sem IO.** Montar o prompt e extrair o SQL da resposta são funções puras e podem morar no core. A chamada ao modelo fica no app.
* **Segurança do editor intacta.** O SQL gerado vai para o buffer do editor e nunca é executado automaticamente. A classificação de statements e as confirmações existentes (§4.7) continuam valendo sem exceção.
* **Custo zero com a feature desligada.** Sem thread, sem modelo em memória, sem impacto no binário além do código HTTP.
* **Licenças.** `cargo deny` aceita só a lista de `deny.toml`. O binário é BUSL-1.1. Pesos de modelo não são distribuídos com o app.
* **Linux x86_64**, empacotamento `.deb`/`.rpm` (e o trabalho de Windows em `docs/windows.md`) não pode ganhar dependência nativa pesada.

### Casos de uso considerados
1. **Texto -> SQL:** o usuário descreve em inglês ou português ("clientes sem pedido em 2026") e recebe um `SELECT` no editor.
2. **Explicar query:** resumo em texto do statement sob o cursor.
3. **Corrigir erro:** dado o erro do driver, sugerir a query corrigida.
4. **Completar inline (ghost text):** sugestão contínua enquanto digita.

O caso 4 exige latência abaixo de ~200 ms por tecla e cancelamento agressivo. Em CPU, com modelos que cabem em notebook, isso não é viável hoje (ver "Desempenho"). O autocomplete determinístico (§D18) já cobre identificadores. **O caso 4 fica fora.** Os casos 2 e 3 reaproveitam toda a infraestrutura do caso 1 e mudam só o prompt.

## Opções Consideradas

### Runtime de inferência
1. **Servidor local externo via HTTP (Ollama ou qualquer servidor compatível com a API OpenAI: `llama-server` do llama.cpp, LM Studio, vLLM).** O usuário instala e escolhe o modelo. O app fala HTTP com `http://127.0.0.1:11434` (ou endpoint configurado).
2. **llama.cpp embutido** via crate de bindings (`llama-cpp-2` ou similar). O app carrega um GGUF e roda a inferência no próprio processo.
3. **`candle` (Hugging Face, Rust puro)** embutido, com modelos GGUF quantizados.
4. **`mistral.rs`** embutido.
5. **API em nuvem** (OpenAI, Anthropic, etc.).
6. **Não fazer.** Manter só o autocomplete determinístico.

### Modelos avaliados (para recomendar ao usuário, não para distribuir)
Licenças e tags foram conferidas em 2026-10-31 nas APIs do Hugging Face (`https://huggingface.co/api/models/<org>/<modelo>`: Qwen/Qwen2.5-Coder-{1.5B,3B,7B}-Instruct (o 3B devolve `license: other`, `license_name: qwen-research`), defog/sqlcoder-7b-2, meta-llama/Llama-3.2-3B-Instruct, microsoft/Phi-4-mini-instruct) e na biblioteca Ollama (`https://ollama.com/library/qwen2.5-coder/tags`, tag `qwen2.5-coder:7b`). Os tamanhos (não reconferidos) são de pesos quantizados em Q4 (aproximados).

| Modelo | Params | Disco/RAM (Q4) | Licença | Observação |
|---|---|---|---|---|
| Qwen2.5-Coder-1.5B-Instruct | 1.5B | ~1 GB | Apache-2.0 | Mínimo viável em CPU. Erra JOINs com mais de 2 tabelas. |
| Qwen2.5-Coder-7B-Instruct | 7B | ~4,5 GB | Apache-2.0 | Melhor equilíbrio qualidade/licença. Pede 16 GB de RAM ou GPU. |
| Qwen2.5-Coder-3B-Instruct | 3B | ~2 GB | Qwen Research (não comercial) | Descartado como recomendação por causa da licença. |
| SQLCoder-7B-2 (Defog) | 7B | ~4,5 GB | CC-BY-SA-4.0 sobre base CodeLlama (Llama 2) | Especializado em SQL, mas licença composta e base antiga. |
| Llama 3.2 3B Instruct | 3B | ~2 GB | Llama Community License | Generalista, SQL mais fraco que Qwen Coder no mesmo porte. |
| Phi-4-mini-instruct | 3.8B | ~2,5 GB | MIT | Alternativa de licença permissiva no porte de 3–4B. |

Como o app não distribui pesos (opção 1), a licença do modelo é responsabilidade do usuário. A UI só sugere padrões.

## Decisão Escolhida
Escolhemos a **Opção 1 (servidor local externo via HTTP)**, com a feature **desligada por padrão** e **opt-in** nas configurações. Modelo sugerido: **`qwen2.5-coder:7b`**, com **`qwen2.5-coder:1.5b`** como opção para máquinas sem GPU ou com menos de 16 GB de RAM. O caso de uso inicial é só **texto -> SQL**.

Motivos:

* **Build e empacotamento não mudam.** As opções 2 e 4 trazem C++/CMake (ou CUDA/Vulkan/ROCm por feature) para o build. Isso multiplica o tempo do CI, complica `.deb`/`.rpm`/Windows e exige escolher backend de GPU em tempo de compilação. A opção 3 evita C++, mas em CPU é mais lenta que llama.cpp e o suporte a GPU no Linux é só CUDA.
* **GPU de graça.** Ollama e llama-server já detectam CUDA/ROCm/Vulkan/Metal. Embutido, o app teria que reimplementar essa matriz.
* **Isolamento de memória.** O modelo (1–5 GB) vive em outro processo. Um OOM ou crash do runtime não derruba o cliente nem as sessões abertas.
* **Pesos fora do binário.** O instalador continua com o tamanho atual. A licença do modelo fica com o usuário.
* **Um protocolo cobre vários runtimes.** Usar o endpoint compatível com OpenAI (`POST /v1/chat/completions`) atende Ollama, llama-server e LM Studio sem código específico.
* **Opção 5 descartada** como padrão porque envia o schema para terceiros. Um endpoint remoto pode ser configurado manualmente, e a UI avisa quando o host não for loopback.
* **Opção 6 descartada** porque o custo da opção 1 é baixo e isolado (ver "Esforço").

### Desenho mínimo
* **`crustdb-core` (puro, testável sem IO):** módulo `ai` com `build_prompt(dialect, schema_ctx, request) -> String` e `extract_sql(response) -> Option<String>` (pega o primeiro bloco ```` ```sql ```` ou o texto inteiro). `schema_ctx` parte do `ObjectTree` (`crates/crustdb-app/src/state/tree.rs`), que guarda os nós de objeto (`ObjectRef`), e do `ColumnCache` (`crates/crustdb-app/src/state/autocomplete.rs`), que guarda `Vec<(ObjectRef, Vec<String>)>`, ou seja, só nomes de coluna, carregados sob demanda. Tipos, PK e FK exigem buscar via `Introspector` sob demanda, o que acrescenta round trips e um passo de montagem no app. O contexto é limitado às tabelas citadas pelo usuário ou às N primeiras, para caber em ~2–4k tokens. Sem tipos novos na API de `Driver`/`Session`.
* **`crustdb-app`:** actor `actors/ai` que recebe `Generate { tab, prompt }` por `tokio::sync::mpsc` e devolve `Chunk`/`Done`/`Error` para o estado da aba. Usa streaming SSE para mostrar progresso e um botão Stop. Stop sinaliza uma flag de cancelamento (`AtomicBool` ou `CancellationToken`) checada entre chunks SSE: o loop sai e fecha a resposta. Drop do `JoinHandle` não serve, pois não interrompe a leitura bloqueante do `ureq`. O timeout do `ureq` (configurável) é o limite superior de espera. Cancelamento imediato de verdade exigiria cliente async; reavaliar no PoC.
* **Cliente HTTP:** reaproveitar uma dependência que já está no grafo. O `ureq` já entra via `sentry`, e o `tokio` com `net` já está no app. Rodar `ureq` em `spawn_blocking` evita uma dependência nova pesada (`reqwest`/`hyper`). Decidir com `cargo tree` na implementação.
* **UI:** campo de prompt (Ctrl+K) acima do editor da aba. A resposta é inserida como seleção no buffer e o usuário revisa antes de executar. Nenhum caminho executa direto.
* **Configuração (persistência existente do app):** `ai.enabled` (false), `ai.endpoint` (`http://127.0.0.1:11434/v1`), `ai.model` (`qwen2.5-coder:7b`), `ai.timeout_secs`. Sem segredo, a menos que o endpoint exija token. Nesse caso, `SecretString` + keyring, como as senhas de conexão.

### Esforço estimado
Estimativa em dias de dev, já contando os testes AAA e os gates.

| Passo | Crates | Esforço |
|---|---|---|
| `ai::build_prompt` + `extract_sql` com testes unitários, mais carga sob demanda de tipos/PK/FK para o `schema_ctx` (risco: pode crescer) | core/app | 1,5 d |
| Settings `ai.*` + persistência + tela de configuração | app/ui | 1 d |
| Actor `ai` com HTTP/SSE, cancelamento, timeout, erros (`thiserror`) | app | 2 d |
| UI do prompt (Ctrl+K), inserção no buffer, estado de loading/erro | ui | 1,5 d |
| Teste de integração do actor contra um servidor HTTP fake (sem modelo) | app | 0,5 d |
| PoC de medição (ver abaixo) + documentação do setup do Ollama | docs | 1 d |
| **Total** | | **~7,5 d** |

Para comparar: a opção 2 (llama.cpp embutido) soma, por estimativa não medida (a confirmar no PoC), de +5 a +10 d de build/CI/empacotamento e matriz de GPU, além de manutenção contínua a cada atualização do llama.cpp.

### Desempenho
**Com a feature desligada (padrão):** impacto nulo. Nenhuma thread ou conexão é criada, e o código extra no binário é só o do actor e da UI (ordem de dezenas de KB, a confirmar).

**Com a feature ligada:** o custo de RAM e CPU fica no processo do runtime, não no `crustdb`. Ordem de grandeza esperada, **a confirmar no PoC** (nenhum número abaixo foi medido neste repositório):

| Cenário | Processar prompt (~2k tokens) | Gerar (~100 tokens) | Latência total estimada |
|---|---|---|---|
| 1.5B Q4, CPU 8 núcleos | ~1–3 s | ~2–5 s | ~3–8 s |
| 7B Q4, CPU 8 núcleos | ~8–20 s | ~10–20 s | ~20–40 s |
| 7B Q4, GPU 8 GB+ | <1 s | ~1–2 s | ~1–3 s |

Conclusões:
* Texto -> SQL sob demanda é aceitável nos três cenários, desde que haja streaming e Stop.
* O ghost text (caso 4) não é aceitável em CPU. Confirma a exclusão.
* O tamanho do contexto de schema domina o tempo em CPU. Por isso o `schema_ctx` é limitado e filtrado.
* A primeira chamada inclui carregar o modelo (~2–10 s, conforme disco e tamanho). O Ollama mantém o modelo carregado por um tempo configurável (`keep_alive`).

**Metodologia do PoC:** 20 perguntas fixas sobre os schemas dos fixtures `crates/crustdb-postgres/tests/fixtures/pg_fixture.sql` e `crates/crustdb-mysql/tests/fixtures/mysql_fixture.sql` (carregados nos containers do `crustdb-testkit`). Para cada modelo da tabela, registrar `prompt_eval_duration`, `eval_count`/`eval_duration` (campos da resposta do Ollama) e a taxa de SQL que (a) passa no parser do `sqlparser` do dialeto e (b) devolve o resultado esperado. Resultados registrados na issue do PoC (`docs/benchs.md` é matriz de features e não serve para isso).

### Consequências Positivas
* Funcionalidade de IA sem dependência nativa nova, sem mudança no build, no empacotamento ou no `cargo deny`.
* Privacidade por padrão (loopback). O schema não sai da máquina.
* A lógica de prompt é pura, fica no core e é testável sem modelo, sem rede e sem Docker.
* O usuário escolhe o modelo e o hardware. O app não fica preso a um modelo específico.
* A política de segurança do editor não muda. O SQL gerado passa pelos mesmos caminhos de confirmação.

### Consequências Negativas / Trade-offs
* Exige que o usuário instale e mantenha um runtime separado (Ollama etc.). É fricção de setup, a ser mitigada com uma mensagem clara quando o endpoint não responder.
* A qualidade depende do modelo escolhido. Modelos de 1.5B erram consultas não triviais, e o usuário precisa revisar sempre.
* Prompt injection pelo conteúdo do schema (por exemplo, um nome de tabela ou de coluna malicioso) pode gerar SQL destrutivo. Mitigação principal: nunca autoexecutar, e o usuário revisa o buffer antes de executar. A política da §4.7 vale como de costume: só os casos que ela sinaliza pedem confirmação (por exemplo, DML com WHERE de estimativa baixa e INSERT de uma tupla têm veredito Run).
* Variação de API entre runtimes (SSE, nomes de campo). Mitigação: usar só o subconjunto OpenAI-compatível.
* Mais uma superfície de configuração e de rede (endpoint arbitrário). Validar a URL e avisar quando o host não for loopback.

### Fora do escopo (YAGNI)
Cada item só vira issue se houver demanda depois do caso 1 estar em uso:
* Ghost text / completar inline (caso 4).
* Explicar query e corrigir erro (casos 2 e 3). São baratos depois do caso 1, mas não entram agora.
* Runtime embutido (llama.cpp/candle/mistral.rs) e download de modelos pelo app.
* Histórico de conversa, multi-turno ou agente que executa queries para "explorar" o banco.
* Embeddings/RAG sobre o schema. Filtrar por nomes citados basta até provar o contrário.
* Fine-tuning ou modelo próprio.
* Provedores em nuvem com SDK dedicado.
