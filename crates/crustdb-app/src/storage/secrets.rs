use std::collections::HashMap;
use std::sync::Mutex;

use crustdb_core::model::ConnectionId;
use secrecy::{ExposeSecret, SecretString};

const SERVICE: &str = "crustdb";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret service unavailable")]
    Unavailable,
    #[error("secret store: {0}")]
    Backend(String),
}

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError>;
    fn set(&self, account: &str, v: &SecretString) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
    fn available(&self) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub enum SecretKind {
    Db,
    SshPassword,
    SshPassphrase,
}

pub fn account(id: ConnectionId, kind: SecretKind) -> String {
    let k = match kind {
        SecretKind::Db => "db",
        SecretKind::SshPassword => "ssh-password",
        SecretKind::SshPassphrase => "ssh-passphrase",
    };
    format!("{}:{k}", id.0)
}

pub struct KeyringStore;

fn backend(e: keyring::Error) -> SecretError {
    SecretError::Backend(e.to_string())
}

fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, account).map_err(backend)
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        match entry(account)?.get_password() {
            Ok(p) => Ok(Some(p.into())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(backend(e)),
        }
    }

    fn set(&self, account: &str, v: &SecretString) -> Result<(), SecretError> {
        entry(account)?
            .set_password(v.expose_secret())
            .map_err(backend)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(backend(e)),
        }
    }

    fn available(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, SecretString>>);

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        let m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Ok(m.get(account).cloned())
    }

    fn set(&self, account: &str, v: &SecretString) -> Result<(), SecretError> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.insert(account.into(), v.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.remove(account);
        Ok(())
    }

    fn available(&self) -> bool {
        true
    }
}

pub struct UnavailableStore;

impl SecretStore for UnavailableStore {
    fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
        Ok(None)
    }

    fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    fn delete(&self, _: &str) -> Result<(), SecretError> {
        Ok(())
    }

    fn available(&self) -> bool {
        false
    }
}

pub fn detect_secret_store() -> Box<dyn SecretStore> {
    // Entry::new also initializes the default store; failure means no Secret Service.
    if keyring::Entry::store_status().is_ok() && keyring::Entry::new(SERVICE, "probe").is_ok() {
        Box::new(KeyringStore)
    } else {
        Box::new(UnavailableStore)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn account_names_match_spec() {
        let id = ConnectionId(uuid::Uuid::nil());
        let n = "00000000-0000-0000-0000-000000000000";
        assert_eq!(account(id, SecretKind::Db), format!("{n}:db"));
        assert_eq!(
            account(id, SecretKind::SshPassword),
            format!("{n}:ssh-password")
        );
        assert_eq!(
            account(id, SecretKind::SshPassphrase),
            format!("{n}:ssh-passphrase")
        );
    }

    #[test]
    fn unavailable_store_never_persists() {
        let s = UnavailableStore;
        assert!(!s.available());
        assert!(matches!(
            s.set("a", &SecretString::from("x")),
            Err(SecretError::Unavailable)
        ));
        assert!(s.get("a").unwrap().is_none());
    }

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::default();
        s.set("a", &SecretString::from("x")).unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().expose_secret(), "x");
        s.delete("a").unwrap();
        assert!(s.get("a").unwrap().is_none());
    }

    #[test]
    fn detect_never_panics() {
        let _ = detect_secret_store();
    }

    #[test]
    #[ignore = "needs secret service"]
    fn keyring_roundtrip() {
        let s = KeyringStore;
        let a = account(ConnectionId(uuid::Uuid::from_u128(7)), SecretKind::Db);
        s.set(&a, &SecretString::from("pw")).unwrap();
        assert_eq!(s.get(&a).unwrap().unwrap().expose_secret(), "pw");
        s.delete(&a).unwrap();
        assert!(s.get(&a).unwrap().is_none());
    }
}
