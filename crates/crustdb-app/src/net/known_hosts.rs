use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

use russh::keys::{Algorithm, HashAlg, PublicKey, known_hosts};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Known,
    Unknown { fingerprint_sha256: String },
    Mismatch,
}

/// `app_file` is read and written; `user_file` (`~/.ssh/known_hosts`) is read-only.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    pub app_file: PathBuf,
    pub user_file: PathBuf,
}

impl KnownHosts {
    /// Fails closed: any entry for the host that does not match the presented key (including a
    /// different key type) or an unreadable/unparseable file is a `Mismatch`, never a prompt.
    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let mut any = false;
        for file in [&self.app_file, &self.user_file] {
            // russh treats an unopenable file as empty; an existing but unreadable one must block.
            if file.exists() && std::fs::File::open(file).is_err() {
                return HostKeyStatus::Mismatch;
            }
            match known_hosts::known_host_keys_path(host, port, file) {
                Ok(keys) => {
                    any |= !keys.is_empty();
                    if keys.iter().any(|(_, k)| k == key) {
                        return HostKeyStatus::Known;
                    }
                }
                Err(_) => return HostKeyStatus::Mismatch,
            }
        }
        if any {
            HostKeyStatus::Mismatch
        } else {
            HostKeyStatus::Unknown {
                fingerprint_sha256: key.fingerprint(HashAlg::Sha256).to_string(),
            }
        }
    }

    /// Key types recorded for the host (unparseable entries are skipped here; `check` fails them).
    pub fn algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        [&self.app_file, &self.user_file]
            .into_iter()
            .filter_map(|f| known_hosts::known_host_keys_path(host, port, f).ok())
            .flatten()
            .map(|(_, k)| k.algorithm())
            .collect()
    }

    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
        if let Some(dir) = self.app_file.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        known_hosts::learn_known_hosts_path(host, port, key, &self.app_file)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILlalEU61txklSBkku8n1ZxRgOGLYm8YtbqsDdo1Ga4j";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";

    fn kh(dir: &tempfile::TempDir) -> KnownHosts {
        KnownHosts {
            app_file: dir.path().join("app/known_hosts"),
            user_file: dir.path().join("user_known_hosts"),
        }
    }

    #[test]
    fn known_hosts_parses_openssh_format() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        let key = PublicKey::from_openssh(KEY_A).unwrap();
        std::fs::write(
            &k.user_file,
            format!("# comment\nother.test {KEY_B}\n[bastion.test]:2222,[10.0.0.1]:2222 {KEY_A} comment\n"),
        )
        .unwrap();

        // Act & Assert
        assert_eq!(k.check("bastion.test", 2222, &key), HostKeyStatus::Known);
        assert_eq!(k.check("10.0.0.1", 2222, &key), HostKeyStatus::Known);
        assert!(matches!(
            k.check("bastion.test", 22, &key),
            HostKeyStatus::Unknown { fingerprint_sha256 } if fingerprint_sha256.starts_with("SHA256:")
        ));
    }

    #[test]
    fn mismatch_detected() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        std::fs::write(&k.user_file, format!("h.test {KEY_B}\n")).unwrap();
        let key = PublicKey::from_openssh(KEY_A).unwrap();

        // Act & Assert
        assert_eq!(k.check("h.test", 22, &key), HostKeyStatus::Mismatch);
    }

    #[test]
    fn different_key_type_for_known_host_is_mismatch() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        let ecdsa = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";
        std::fs::write(&k.user_file, format!("h.test {ecdsa}\n")).unwrap();
        let key = PublicKey::from_openssh(KEY_A).unwrap();

        // Act & Assert
        assert_eq!(k.check("h.test", 22, &key), HostKeyStatus::Mismatch);
        assert_eq!(k.algorithms("h.test", 22).len(), 1);
    }

    #[test]
    fn malformed_file_fails_closed() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        let key = PublicKey::from_openssh(KEY_A).unwrap();
        std::fs::write(&k.user_file, "h.test ssh-ed25519 !!!\n").unwrap();

        // Act & Assert
        assert_eq!(k.check("h.test", 22, &key), HostKeyStatus::Mismatch);
        std::fs::write(&k.user_file, b"\xff\xfe\n").unwrap();
        assert_eq!(k.check("h.test", 22, &key), HostKeyStatus::Mismatch);
    }

    #[test]
    fn unreadable_existing_file_fails_closed() {
        // Arrange
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        std::fs::write(&k.user_file, "").unwrap();
        std::fs::set_permissions(&k.user_file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&k.user_file).is_ok() {
            return; // running as root
        }
        let key = PublicKey::from_openssh(KEY_A).unwrap();

        // Act & Assert
        assert_eq!(k.check("h.test", 22, &key), HostKeyStatus::Mismatch);
    }

    #[test]
    fn add_writes_app_file_only_and_is_then_known() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let k = kh(&dir);
        let key = PublicKey::from_openssh(KEY_A).unwrap();

        // Act
        k.add("h.test", 2222, &key).unwrap();

        // Assert
        assert_eq!(k.check("h.test", 2222, &key), HostKeyStatus::Known);
        assert!(!k.user_file.exists());
    }
}
