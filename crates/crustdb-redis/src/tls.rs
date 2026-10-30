//! TLS client configuration for the Redis connection.
//!
//! ponytail: duplicated from `crustdb-postgres` on purpose (YAGNI); extract to a shared crate if a third driver needs it.

use std::sync::Arc;

use crustdb_core::driver::{TlsConfig, TlsMode};
use crustdb_core::error::{DriverError, ErrorKind};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};

pub(crate) fn err(kind: ErrorKind, message: impl Into<String>) -> DriverError {
    DriverError {
        kind,
        message: message.into(),
    }
}

/// `VerifyCa` is enforced as `VerifyFull`: the client library cannot skip only the name check
/// (fail-safe: never weaker than what the user asked for).
pub(crate) fn effective_mode(mode: TlsMode) -> TlsMode {
    match mode {
        TlsMode::VerifyCa => TlsMode::VerifyFull,
        m => m,
    }
}

/// `Require`: encrypts but accepts any certificate. Handshake signatures are still checked.
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
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
pub(crate) fn client_config(
    tls: &TlsConfig,
) -> Result<Option<Arc<rustls::ClientConfig>>, DriverError> {
    let mode = effective_mode(tls.mode);
    if mode == TlsMode::Disable {
        return Ok(None);
    }
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| err(ErrorKind::Tls, e.to_string()))?;
    let cfg = if mode == TlsMode::VerifyFull {
        let verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots(tls.ca_path.as_deref())?),
            provider,
        )
        .build()
        .map_err(|e| err(ErrorKind::Tls, e.to_string()))?;
        builder.with_webpki_verifier(verifier)
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    }
    .with_no_client_auth();
    Ok(Some(Arc::new(cfg)))
}

/// D39: a server without TLS 1.2+ gets a fixed, actionable message.
pub(crate) fn tls_error(e: &rustls::Error, message: String) -> DriverError {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn cfg(mode: TlsMode) -> TlsConfig {
        TlsConfig {
            mode,
            ca_path: None,
        }
    }

    #[test]
    fn verify_ca_is_enforced_as_verify_full() {
        // Act & Assert
        assert_eq!(effective_mode(TlsMode::VerifyCa), TlsMode::VerifyFull);
        assert_eq!(effective_mode(TlsMode::VerifyFull), TlsMode::VerifyFull);
        assert_eq!(effective_mode(TlsMode::Require), TlsMode::Require);
        assert_eq!(effective_mode(TlsMode::Disable), TlsMode::Disable);
    }

    #[test]
    fn require_builds_config_without_verification() {
        // Arrange
        let garbage = CertificateDer::from(vec![0u8; 8]);
        let name = ServerName::try_from("localhost").unwrap();
        let verifier = NoVerify(Arc::new(aws_lc_rs::default_provider()));

        // Act
        let built = client_config(&cfg(TlsMode::Require)).unwrap();
        let verdict = verifier.verify_server_cert(&garbage, &[], &name, &[], UnixTime::now());

        // Assert
        assert!(built.is_some());
        assert!(verdict.is_ok());
    }

    #[test]
    fn disable_builds_no_tls() {
        // Act & Assert
        assert!(client_config(&cfg(TlsMode::Disable)).unwrap().is_none());
    }

    #[test]
    fn old_tls_maps_to_d39_message() {
        // Arrange
        use rustls::{AlertDescription, PeerIncompatible};
        const D39: &str = "server does not support TLS 1.2+; use an SSH tunnel or disable TLS";

        // Act & Assert
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
