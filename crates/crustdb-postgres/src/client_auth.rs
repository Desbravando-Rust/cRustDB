use std::path::Path;

use crustdb_core::driver::TlsConfig;
use crustdb_core::error::{DriverError, ErrorKind};
use rustls::pki_types::pem::{Error as PemError, PemObject};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::connect::err;

pub(crate) type ClientAuth = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

/// Client certificate chain and key (PEM) for mTLS; `None` when neither path is set.
// ponytail: duplicated in crustdb-mysql (core does no IO); share a crate if a third driver needs it.
pub(crate) fn client_auth(tls: &TlsConfig) -> Result<Option<ClientAuth>, DriverError> {
    let (cert, key) = match (&tls.client_cert_path, &tls.client_key_path) {
        (None, None) => return Ok(None),
        (Some(c), Some(k)) => (c, k),
        _ => {
            return Err(err(
                ErrorKind::Tls,
                "client certificate and client key must be set together",
            ));
        }
    };
    Ok(Some((load_chain(cert)?, load_key(key)?)))
}

fn load_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>, DriverError> {
    let bad = |e: PemError| {
        err(
            ErrorKind::Tls,
            format!("cannot read client certificate {}: {e}", path.display()),
        )
    };
    let chain = CertificateDer::pem_file_iter(path)
        .map_err(bad)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(bad)?;
    if chain.is_empty() {
        return Err(err(
            ErrorKind::Tls,
            format!("no certificate in {}", path.display()),
        ));
    }
    Ok(chain)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, DriverError> {
    PrivateKeyDer::from_pem_file(path).map_err(|e| {
        let message = match e {
            PemError::NoItemsFound => format!(
                "no unencrypted private key in {}; password-protected keys are not supported",
                path.display()
            ),
            e => format!("cannot read client key {}: {e}", path.display()),
        };
        err(ErrorKind::Tls, message)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;

    use crustdb_core::driver::TlsMode;

    use super::*;

    fn tls(cert: Option<PathBuf>, key: Option<PathBuf>) -> TlsConfig {
        TlsConfig {
            mode: TlsMode::Require,
            ca_path: None,
            client_cert_path: cert,
            client_key_path: key,
        }
    }

    #[test]
    fn no_paths_means_no_client_auth() {
        // Act & Assert
        assert!(client_auth(&tls(None, None)).unwrap().is_none());
    }

    #[test]
    fn loads_cert_and_key() {
        // Arrange
        let c = crustdb_testkit::certs_for(&["localhost"]);

        // Act
        let auth = client_auth(&tls(
            Some(c.dir.path().join("client.crt")),
            Some(c.dir.path().join("client.key")),
        ))
        .unwrap();

        // Assert
        assert_eq!(auth.unwrap().0.len(), 1);
    }

    #[test]
    fn rejects_cert_without_key() {
        // Act
        let e = client_auth(&tls(Some("/c.crt".into()), None))
            .err()
            .unwrap();

        // Assert
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(e.message.contains("together"), "{}", e.message);
    }

    #[test]
    fn unreadable_file_error_names_path() {
        // Arrange
        let c = crustdb_testkit::certs_for(&["localhost"]);
        let key = c.dir.path().join("client.key");

        // Act
        let e = client_auth(&tls(Some("/nope/client.crt".into()), Some(key)))
            .err()
            .unwrap();

        // Assert
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(e.message.contains("/nope/client.crt"), "{}", e.message);
    }

    #[test]
    fn invalid_pem_is_tls_error() {
        // Arrange
        let c = crustdb_testkit::certs_for(&["localhost"]);
        let bad = c.dir.path().join("bad.crt");
        std::fs::write(&bad, "not a pem").unwrap();
        let key = c.dir.path().join("client.key");

        // Act
        let e = client_auth(&tls(Some(bad.clone()), Some(key)))
            .err()
            .unwrap();

        // Assert
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(e.message.contains(&bad.display().to_string()));
    }

    #[test]
    fn encrypted_key_asks_for_unencrypted() {
        // Arrange
        let c = crustdb_testkit::certs_for(&["localhost"]);
        let enc = c.dir.path().join("enc.key");
        std::fs::write(
            &enc,
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n",
        )
        .unwrap();

        // Act
        let e = client_auth(&tls(Some(c.dir.path().join("client.crt")), Some(enc)))
            .err()
            .unwrap();

        // Assert
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(e.message.contains("unencrypted"), "{}", e.message);
    }
}
