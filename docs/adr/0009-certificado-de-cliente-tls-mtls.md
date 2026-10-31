# Certificado de cliente TLS (mTLS) em TlsConfig

* **Status:** Aceito
* **Data:** 2026-10-31
* **Issue Relacionada:** #21

## Contexto e Problema
`TlsConfig` (`crustdb-core/src/driver.rs`, API pública do core e parte do schema de `connections.json`) só tem `mode` e `ca_path`. Servidores PostgreSQL/MySQL que exigem certificado de cliente (mTLS) não conectam. É preciso guardar o par certificado/chave (PEM) por conexão, carregá-lo no `rustls::ClientConfig` dos drivers, ler o par da URL (`sslcert`/`sslkey`, `ssl-cert`/`ssl-key`) e manter configs já salvas carregando sem mudança. O core não faz IO, então a leitura dos arquivos fica nos drivers.

## Opções Consideradas
1. Dois campos planos em `TlsConfig`: `client_cert_path: Option<PathBuf>` e `client_key_path: Option<PathBuf>`, ambos `#[serde(default, skip_serializing_if = "Option::is_none")]`. A regra "os dois ou nenhum" é validada no formulário e de novo no driver (erro `ErrorKind::Tls`).
2. Um campo `client_cert: Option<ClientCert { cert_path, key_path }>`: estado inválido não representável, mas o formulário edita `draft.tls` direto e passaria a precisar de rascunho separado para cada caminho, e a URL pode trazer só um dos dois.
3. Novo `ErrorKind::ClientCert` para falhas de leitura/PEM.

## Decisão Escolhida
Escolhemos a **Opção 1** porque o formulário e o import de URL continuam editando `Option<PathBuf>` com o `path_edit` existente, `connections.json` antigo desserializa igual (campos ausentes = `None`) e um config sem certificado serializa byte a byte como antes (sem bump de `version`). A Opção 3 foi descartada: `ErrorKind::Tls` + mensagem com o caminho já é o erro tipado que a UI exibe; nova variante mudaria a API pública sem consumidor.

Detalhes:
* Drivers PostgreSQL e MySQL carregam a cadeia (`CertificateDer::pem_file_iter`) e a chave (`PrivateKeyDer::from_pem_file`) com `rustls::pki_types`. Arquivo ilegível, PEM sem item ou chave criptografada viram `DriverError { kind: Tls, message }` com o caminho; chave com senha recebe mensagem pedindo chave sem senha. Sem panic.
* PostgreSQL: `with_client_auth_cert(chain, key)` no lugar de `with_no_client_auth()`.
* MySQL: `SslOpts::with_client_identity(ClientIdentity::new(..))` do `mysql_async` (que chama `with_client_auth_cert` internamente), alimentado com os bytes já validados pelo driver, para o erro sair tipado antes do handshake.
* SQLite limpa os campos (como já faz com `ca_path`). Redis fica fora desta issue: o formulário oculta os campos e limpa os valores para Redis; mTLS no Redis vira issue própria.
* A leitura dos arquivos acontece dentro de `connect` (tarefa tokio), nunca na UI thread.

### Consequências Positivas
* Sem dependência nova (`rustls` já está nos dois drivers; `rcgen` já está no `crustdb-testkit`).
* Compatível com configs antigos e com a forma de serializar atual.
* Erro com caminho do arquivo, exibido pela UI como os demais erros de TLS.

### Consequências Negativas / Trade-offs
* A regra "os dois ou nenhum" não é garantida pelo tipo; vive em duas checagens (formulário e driver).
* Todos os literais `TlsConfig { .. }` do workspace (testes inclusos) ganham dois campos `None`.
* O carregamento PEM é duplicado em `crustdb-postgres` e `crustdb-mysql` (~15 linhas), pois o core não faz IO e não há crate compartilhado entre drivers.
* Fora de escopo: PKCS#12, chave com senha, certificados do keyring do sistema, mTLS no Redis.
