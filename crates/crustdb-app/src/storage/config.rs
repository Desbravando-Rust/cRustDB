use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crustdb_core::driver::TlsConfig;
use crustdb_core::model::{ConnectionId, DriverKind};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SshAuth {
    Password,
    Key { key_path: PathBuf },
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: ConnectionId,
    pub name: String,
    pub driver: DriverKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: Option<String>,
    pub save_password: bool,
    pub tls: TlsConfig,
    pub ssh: Option<SshConfig>,
    pub page_size: u32,
    pub editor_row_limit: u32,
    pub safety_threshold: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config is read-only (unsupported version)")]
    ReadOnly,
    #[error("config io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config serialization: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub connections: Vec<ConnectionConfig>,
    pub read_only: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

pub struct ConfigStore {
    pub path: PathBuf,
}

#[derive(Deserialize)]
struct Versioned {
    version: u32,
    connections: serde_json::Value,
}

impl ConfigStore {
    pub fn load(&self) -> Result<LoadedConfig, ConfigError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadedConfig {
                    connections: vec![],
                    read_only: false,
                    warning: None,
                });
            }
            Err(e) => return Err(e.into()),
        };
        let Ok(doc) = serde_json::from_str::<Versioned>(&text) else {
            return self.backup_and_reset();
        };
        if doc.version != VERSION {
            return Ok(LoadedConfig {
                connections: serde_json::from_value(doc.connections).unwrap_or_default(),
                read_only: true,
                warning: Some(format!(
                    "connections.json has unsupported version {}; opened read-only",
                    doc.version
                )),
            });
        }
        match serde_json::from_value(doc.connections) {
            Ok(connections) => Ok(LoadedConfig {
                connections,
                read_only: false,
                warning: None,
            }),
            Err(_) => self.backup_and_reset(),
        }
    }

    fn backup_and_reset(&self) -> Result<LoadedConfig, ConfigError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut bak = self.path.clone().into_os_string();
        bak.push(format!(".bak-{ts}"));
        fs::rename(&self.path, &bak)?;
        Ok(LoadedConfig {
            connections: vec![],
            read_only: false,
            warning: Some(format!(
                "connections.json was invalid; backed up to {}",
                PathBuf::from(bak).display()
            )),
        })
    }

    pub fn save(&self, c: &[ConnectionConfig]) -> Result<(), ConfigError> {
        if self.load()?.read_only {
            return Err(ConfigError::ReadOnly);
        }
        let json = serde_json::to_vec_pretty(
            &serde_json::json!({ "version": VERSION, "connections": c }),
        )?;
        if let Some(dir) = self.path.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

pub fn validate(c: &ConnectionConfig, others: &[ConnectionConfig]) -> Result<(), Vec<FieldError>> {
    let mut errs = Vec::new();
    let mut bad = |field, message: &str| {
        errs.push(FieldError {
            field,
            message: message.into(),
        })
    };
    if c.name.trim().is_empty() {
        bad("name", "name is required");
    } else if others.iter().any(|o| o.id != c.id && o.name == c.name) {
        bad("name", "name already in use");
    }
    if c.driver == DriverKind::Sqlite {
        // A file, not a server: host and port are unused.
        if c.database.as_deref().is_none_or(|d| d.trim().is_empty()) {
            bad("database", "database file is required");
        }
    } else {
        if c.host.trim().is_empty() {
            bad("host", "host is required");
        }
        if c.port == 0 {
            bad("port", "port must be 1-65535");
        }
    }
    if !(10..=10_000).contains(&c.page_size) {
        bad("page_size", "page size must be 10-10000");
    }
    if !(10..=100_000).contains(&c.editor_row_limit) {
        bad("editor_row_limit", "editor row limit must be 10-100000");
    }
    if c.safety_threshold < 1 {
        bad("safety_threshold", "safety threshold must be at least 1");
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const SPEC: &str = r#"{ "version": 1, "connections": [ {
      "id": "7f3c1a2e-0000-4000-8000-000000000001", "name": "prod-pg", "driver": "postgres",
      "host": "db.example.com", "port": 5432, "user": "app", "database": "app_db",
      "save_password": true,
      "tls": { "mode": "verify-full", "ca_path": null },
      "ssh": { "host": "bastion.example.com", "port": 22, "user": "ops",
               "auth": { "kind": "key", "key_path": "/home/u/.ssh/id_ed25519" } },
      "page_size": 300, "editor_row_limit": 1000, "safety_threshold": 1000 } ] }"#;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let t = tempfile::tempdir().unwrap();
        let s = ConfigStore {
            path: t.path().join("connections.json"),
        };
        (t, s)
    }

    fn sample() -> ConnectionConfig {
        let v: serde_json::Value = serde_json::from_str(SPEC).unwrap();
        serde_json::from_value(v["connections"][0].clone()).unwrap()
    }

    #[test]
    fn config_roundtrip_matches_spec_json() {
        let (_t, s) = store();
        fs::write(&s.path, SPEC).unwrap();
        let loaded = s.load().unwrap();
        assert_eq!(loaded.connections.len(), 1);
        s.save(&loaded.connections).unwrap();
        let a: serde_json::Value = serde_json::from_str(SPEC).unwrap();
        let b: serde_json::Value = serde_json::from_slice(&fs::read(&s.path).unwrap()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn config_roundtrip_has_no_secrets() {
        let (_t, s) = store();
        s.save(&[sample()]).unwrap();
        assert!(
            !fs::read_to_string(&s.path)
                .unwrap()
                .contains("\"password\"")
        );
    }

    #[test]
    fn config_written_atomically_with_0600() {
        let (t, s) = store();
        s.save(&[sample()]).unwrap();
        assert_eq!(
            s.path.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!t.path().join("connections.json.tmp").exists());
    }

    #[test]
    fn unknown_version_opens_read_only() {
        let (_t, s) = store();
        let v1: serde_json::Value = serde_json::from_str(SPEC).unwrap();
        let raw = serde_json::json!({"version": 99, "connections": v1["connections"]}).to_string();
        fs::write(&s.path, &raw).unwrap();
        let l = s.load().unwrap();
        assert!(l.read_only && l.warning.is_some());
        assert_eq!(l.connections.len(), 1);
        assert!(matches!(s.save(&[]), Err(ConfigError::ReadOnly)));
        assert_eq!(fs::read_to_string(&s.path).unwrap(), raw);
    }

    #[test]
    fn stale_tmp_with_open_mode_does_not_leak_into_final() {
        let (t, s) = store();
        let tmp = t.path().join("connections.json.tmp");
        fs::write(&tmp, "x").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();
        s.save(&[sample()]).unwrap();
        assert_eq!(
            s.path.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn invalid_json_backed_up_and_empty() {
        let (t, s) = store();
        fs::write(&s.path, "{not json").unwrap();
        let l = s.load().unwrap();
        assert!(l.connections.is_empty() && !l.read_only && l.warning.is_some());
        let baks = fs::read_dir(t.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("connections.json.bak-")
            })
            .count();
        assert_eq!(baks, 1);
    }

    #[test]
    fn validate_rejects_duplicate_name_and_bad_ranges() {
        let ok = sample();
        assert!(validate(&ok, &[]).is_ok());
        let mut other = ok.clone();
        other.id =
            ConnectionId(serde_json::from_str("\"7f3c1a2e-0000-4000-8000-000000000002\"").unwrap());
        let mut bad = ok;
        bad.port = 0;
        bad.page_size = 5;
        bad.editor_row_limit = 200_000;
        bad.safety_threshold = 0;
        let errs = validate(&bad, &[other]).unwrap_err();
        let fields: Vec<_> = errs.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            [
                "name",
                "port",
                "page_size",
                "editor_row_limit",
                "safety_threshold"
            ]
        );
    }

    #[test]
    fn sqlite_requires_file_not_host_or_port() {
        let mut c = sample();
        c.driver = DriverKind::Sqlite;
        c.host = String::new();
        c.port = 0;
        c.database = Some("/tmp/a.db".into());
        assert!(validate(&c, &[]).is_ok());
        c.database = None;
        let errs = validate(&c, &[]).unwrap_err();
        let fields: Vec<_> = errs.iter().map(|e| e.field).collect();
        assert_eq!(fields, ["database"]);
    }
}
