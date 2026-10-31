use std::path::Path;

use crustdb_core::driver::TlsConfig;
use crustdb_core::error::{DriverError, ErrorKind};
use mysql_async::ClientIdentity;
use rustls::pki_types::pem::{Error as PemError, PemObject};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::connect::err;

/// Validated client certificate chain and key (PEM bytes); `None` when neither path is set.
// ponytail: duplicated in crustdb-postgres (core does no IO); share a crate if a third driver needs it.
pub(crate) fn client_identity(tls: &TlsConfig) -> Result<Option<ClientIdentity>, DriverError> {
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
    let cert_pem = read(cert, "client certificate")?;
    let key_pem = read(key, "client key")?;
    let bad_cert = |e: PemError| {
        err(
            ErrorKind::Tls,
            format!("cannot read client certificate {}: {e}", cert.display()),
        )
    };
    let chain = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(bad_cert)?;
    if chain.is_empty() {
        return Err(err(
            ErrorKind::Tls,
            format!("no certificate in {}", cert.display()),
        ));
    }
    PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| {
        let message = match e {
            PemError::NoItemsFound => format!(
                "no unencrypted private key in {}; password-protected keys are not supported",
                key.display()
            ),
            e => format!("cannot read client key {}: {e}", key.display()),
        };
        err(ErrorKind::Tls, message)
    })?;
    Ok(Some(ClientIdentity::new(cert_pem.into(), key_pem.into())))
}

fn read(path: &Path, what: &str) -> Result<Vec<u8>, DriverError> {
    std::fs::read(path).map_err(|e| {
        err(
            ErrorKind::Tls,
            format!("cannot read {what} {}: {e}", path.display()),
        )
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
    fn no_paths_means_no_identity() {
        // Act & Assert
        assert!(client_identity(&tls(None, None)).unwrap().is_none());
    }

    #[test]
    fn loads_cert_and_key() {
        // Arrange
        let c = crustdb_testkit::certs_for(&["localhost"]);

        // Act
        let id = client_identity(&tls(
            Some(c.dir.path().join("client.crt")),
            Some(c.dir.path().join("client.key")),
        ))
        .unwrap();

        // Assert
        assert!(id.is_some());
    }

    #[test]
    fn rejects_cert_without_key() {
        // Act
        let e = client_identity(&tls(Some("/c.crt".into()), None))
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
        let e = client_identity(&tls(Some("/nope/client.crt".into()), Some(key)))
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
        let e = client_identity(&tls(Some(bad.clone()), Some(key)))
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
        let e = client_identity(&tls(Some(c.dir.path().join("client.crt")), Some(enc)))
            .err()
            .unwrap();

        // Assert
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(e.message.contains("unencrypted"), "{}", e.message);
    }
}
