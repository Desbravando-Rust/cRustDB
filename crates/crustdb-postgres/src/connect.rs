use std::sync::Arc;

use crustdb_core::driver::{ConnectTarget, SessionPurpose, TlsMode};
use crustdb_core::error::{DriverError, ErrorKind};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};
use secrecy::ExposeSecret;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Config, NoTls, error::SqlState};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::session::PgSession;

pub(crate) fn err(kind: ErrorKind, message: impl Into<String>) -> DriverError {
    DriverError {
        kind,
        message: message.into(),
    }
}

/// Verifies handshake signatures but accepts any certificate (`Require`), or any
/// certificate chain-valid for a different name (`VerifyCa`, when `inner` is set).
#[derive(Debug)]
struct Lenient {
    provider: Arc<CryptoProvider>,
    inner: Option<Arc<WebPkiServerVerifier>>,
}

impl ServerCertVerifier for Lenient {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let Some(inner) = &self.inner else {
            return Ok(ServerCertVerified::assertion());
        };
        match inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            r => r,
        }
    }

    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            d,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            d,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn roots(ca_path: Option<&std::path::Path>) -> Result<RootCertStore, DriverError> {
    let mut store = RootCertStore::empty();
    let tls_err = |e: &dyn std::fmt::Display| {
        err(ErrorKind::Tls, format!("cannot load CA certificates: {e}"))
    };
    if let Some(p) = ca_path {
        for c in CertificateDer::pem_file_iter(p).map_err(|e| tls_err(&e))? {
            store
                .add(c.map_err(|e| tls_err(&e))?)
                .map_err(|e| tls_err(&e))?;
        }
    } else {
        store.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    }
    Ok(store)
}

/// `None` for `Disable`.
fn tls_connector(t: &ConnectTarget) -> Result<Option<MakeRustlsConnect>, DriverError> {
    if t.tls.mode == TlsMode::Disable {
        return Ok(None);
    }
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| err(ErrorKind::Tls, e.to_string()))?;
    let webpki = || -> Result<Arc<WebPkiServerVerifier>, DriverError> {
        WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots(t.tls.ca_path.as_deref())?),
            provider.clone(),
        )
        .build()
        .map_err(|e| err(ErrorKind::Tls, e.to_string()))
    };
    let cfg = match t.tls.mode {
        TlsMode::VerifyFull => builder.with_webpki_verifier(webpki()?),
        TlsMode::VerifyCa => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Lenient {
                provider: provider.clone(),
                inner: Some(webpki()?),
            })),
        _ => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Lenient {
                provider,
                inner: None,
            })),
    }
    .with_no_client_auth();
    Ok(Some(MakeRustlsConnect::new(cfg)))
}

pub(crate) fn map_err(e: tokio_postgres::Error) -> DriverError {
    let message = e
        .as_db_error()
        .map_or_else(|| e.to_string(), |d| d.message().to_string());
    if let Some(db) = e.as_db_error() {
        let kind = if matches!(
            *db.code(),
            SqlState::INVALID_PASSWORD | SqlState::INVALID_AUTHORIZATION_SPECIFICATION
        ) {
            ErrorKind::Auth
        } else {
            let position = match db.position() {
                Some(tokio_postgres::error::ErrorPosition::Original(n)) => Some(*n as usize),
                _ => None,
            };
            ErrorKind::Query {
                code: Some(db.code().code().to_string()),
                position,
            }
        };
        return err(kind, message);
    }
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&e);
    while let Some(c) = cur {
        let io = c.downcast_ref::<std::io::Error>();
        let tls = c.downcast_ref::<rustls::Error>().or_else(|| {
            io.and_then(|i| i.get_ref())
                .and_then(|i| i.downcast_ref::<rustls::Error>())
        });
        if let Some(t) = tls {
            return tls_error(t, message_with_cause(&e));
        }
        if io.is_some_and(|i| i.kind() == std::io::ErrorKind::TimedOut) {
            return err(ErrorKind::Timeout, "connection timed out");
        }
        cur = c.source();
    }
    err(ErrorKind::Connect, message_with_cause(&e))
}

/// D39: a server without TLS 1.2+ gets a fixed, actionable message.
fn tls_error(e: &rustls::Error, message: String) -> DriverError {
    use rustls::{
        AlertDescription::ProtocolVersion, Error::*,
        PeerIncompatible::ServerDoesNotSupportTls12Or13,
    };
    let no_tls12 = matches!(
        e,
        PeerIncompatible(ServerDoesNotSupportTls12Or13) | AlertReceived(ProtocolVersion)
    );
    let message = if no_tls12 {
        "server does not support TLS 1.2+; use an SSH tunnel or disable TLS".into()
    } else {
        message
    };
    err(ErrorKind::Tls, message)
}

/// `tokio_postgres::Error` displays only a generic text; append the root cause.
fn message_with_cause(e: &tokio_postgres::Error) -> String {
    let mut s = e.to_string();
    let mut cur = std::error::Error::source(e);
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

pub(crate) async fn connect(
    t: &ConnectTarget,
    purpose: SessionPurpose,
) -> Result<PgSession, DriverError> {
    let tls = tls_connector(t)?;
    let mut cfg = Config::new();
    cfg.host(&t.host)
        .user(&t.user)
        .connect_timeout(t.connect_timeout)
        .ssl_mode(if tls.is_some() {
            SslMode::Require
        } else {
            SslMode::Disable
        });
    match t.connect_addr {
        Some(a) => cfg.hostaddr(a.ip()).port(a.port()),
        None => cfg.port(t.port),
    };
    if let Some(p) = &t.password {
        cfg.password(p.expose_secret());
    }
    if let Some(d) = &t.database {
        cfg.dbname(d);
    }
    let timeout = || err(ErrorKind::Timeout, "connection timed out");
    let work = async {
        let client = match &tls {
            Some(tls) => {
                let (c, conn) = cfg.connect(tls.clone()).await.map_err(map_err)?;
                tokio::spawn(conn);
                c
            }
            None => {
                let (c, conn) = cfg.connect(NoTls).await.map_err(map_err)?;
                tokio::spawn(conn);
                c
            }
        };
        if purpose != SessionPurpose::Editor {
            client
                .batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
                .await
                .map_err(map_err)?;
        }
        let raw = client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(map_err)?
            .get::<_, String>(0);
        Ok(PgSession::new(client, raw, tls))
    };
    tokio::time::timeout(t.connect_timeout, work)
        .await
        .map_err(|_| timeout())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_tls_maps_to_d39_message() {
        use rustls::{AlertDescription, PeerIncompatible};
        const D39: &str = "server does not support TLS 1.2+; use an SSH tunnel or disable TLS";
        for e in [
            rustls::Error::PeerIncompatible(PeerIncompatible::ServerDoesNotSupportTls12Or13),
            rustls::Error::AlertReceived(AlertDescription::ProtocolVersion),
        ] {
            let d = tls_error(&e, "x".into());
            assert_eq!((d.kind, d.message.as_str()), (ErrorKind::Tls, D39));
        }
        let e = rustls::Error::PeerIncompatible(PeerIncompatible::NoCipherSuitesInCommon);
        assert_eq!(tls_error(&e, "x".into()).message, "x");
    }
}
