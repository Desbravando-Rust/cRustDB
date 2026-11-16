# Egress de rede para aviso de release e feed do livro

* **Status:** Aceito
* **Data:** 2026-11-01
* **Issue Relacionada:** #230

## Contexto e Problema
A #227 deixou dois itens que exigem rede: aviso de release nova do GitHub na sidebar e bloco do livro
dinâmico (capítulo aleatório de `https://desbravandorust.com.br/feed.json`, fallback no parágrafo
estático). Hoje o app só faz egress para o banco do usuário e para o Sentry (panics). É preciso
decidir o cliente HTTP, onde as requisições rodam (a UI `egui` nunca bloqueia) e como o usuário
desliga o egress.

## Opções Consideradas
1. `ureq` 3 (já transitiva via `sentry`, mesmo backend `rustls`) chamado em uma `std::thread` única
   no startup, publicando o resultado em `OnceLock`s compartilhados e pedindo repaint.
2. `reqwest` assíncrono como novo comando/ator do `Runtime` tokio existente.
3. Não ter rede: link estático para a página de releases.

## Decisão Escolhida
Escolhemos a **Opção 1** porque `ureq` não adiciona árvore nova (já está no `Cargo.lock` via
`sentry`), é síncrono e cabe numa thread descartável de ~1 s, sem acoplar o `Runtime` de drivers
(protocolo `Command`/`Event`) a algo que não é banco. Detalhes:

* Uma thread, duas GETs (GitHub `releases/latest` e `feed.json`), timeout global curto (5 s), sem
  retry; falha é silenciosa (log `debug`) e a UI mantém o fallback.
* Só em `CrustApp::new` (binário real); `CrustApp::with_ctx` (testes de UI) nunca faz rede.
* Opt-out: `CRUSTDB_OFFLINE=1` desliga ambas as requisições (vazio ou `0` equivale a não definido). Nenhum dado do usuário é enviado; só
  `User-Agent: cRustDB/<versão>`. Documentado no README.
* Parsing/comparação de versão são funções puras testáveis; sem crate `semver` nem `rand`
  (índice aleatório via `SystemTime` nanos).

### Consequências Positivas
* Zero bloqueio da UI thread; zero dependência transitiva nova.
* Rede isolada em um módulo pequeno, fácil de remover ou trocar.

### Consequências Negativas / Trade-offs
* Egress por padrão no startup (opt-out, não opt-in); um toggle na UI fica para quando pedirem.
* Sem cache: uma requisição por abertura do app; rate limit anônimo do GitHub (60/h por IP) basta.
* `ureq` vira dependência direta e precisa acompanhar a versão usada pelo `sentry`.

## Adendo (#244): feed do blog

O painel "From the Blog" mostra o post mais recente. Mesma thread, uma terceira GET (mesmo timeout, sem
retry, mesmo opt-out `CRUSTDB_OFFLINE`), sem dependência nova. Contrato que o site publica em
`https://desbravandorust.com.br/blog.json` (subconjunto de JSON Feed 1.1):

```json
{"items":[{"title":"Why Indexing Matters","url":"https://desbravandorust.com.br/blog/indexing",
  "summary":"Resumo curto...","date_published":"2026-11-10T12:00:00Z"}]}
```

* `title` e `url` obrigatórios; item sem eles, ou com `url` fora de `http://`/`https://`, é descartado
  (feed remoto não confiável; o egui abriria qualquer esquema).
* `summary` (ou, na falta, `content_text`) é opcional; o app corta em 240 caracteres (limite de caractere,
  nunca no meio de um UTF-8) e acrescenta `…`.
* Vale o item de maior `date_published` (ISO 8601, comparado como texto); sem datas, o primeiro.
* Sem post (offline, opt-out, feed vazio ou inválido): linha estática com link para
  `https://desbravandorust.com.br/`. O cartão do livro ("DESTAQUE", em português como no protótipo) é estático.

