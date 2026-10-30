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
pub struct Label {
    pub name: String,
    pub color: [u8; 3],
}

fn default_labels() -> Vec<Label> {
    [
        ("local", [76, 175, 80]),
        ("dev", [33, 150, 243]),
        ("staging", [255, 193, 7]),
        ("production", [244, 67, 54]),
    ]
    .map(|(n, color)| Label {
        name: n.into(),
        color,
    })
    .to_vec()
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
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
    pub labels: Vec<Label>,
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
    #[serde(default = "default_labels")]
    labels: Vec<Label>,
}

impl ConfigStore {
    pub fn load(&self) -> Result<LoadedConfig, ConfigError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadedConfig {
                    connections: vec![],
                    labels: default_labels(),
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
                labels: doc.labels,
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
                labels: doc.labels,
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
            labels: default_labels(),
            read_only: false,
            warning: Some(format!(
                "connections.json was invalid; backed up to {}",
                PathBuf::from(bak).display()
            )),
        })
    }

    pub fn save(&self, c: &[ConnectionConfig], labels: &[Label]) -> Result<(), ConfigError> {
        if self.load()?.read_only {
            return Err(ConfigError::ReadOnly);
        }
        let json = serde_json::to_vec_pretty(
            &serde_json::json!({ "version": VERSION, "connections": c, "labels": labels }),
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

/// Renames a label in the catalog and in every connection that uses it.
pub fn rename_label(conns: &mut [ConnectionConfig], cat: &mut [Label], old: &str, new: &str) {
    for l in cat.iter_mut().filter(|l| l.name == old) {
        l.name = new.into();
    }
    for n in conns.iter_mut().flat_map(|c| c.labels.iter_mut()) {
        if n == old {
            *n = new.into();
        }
    }
}

pub fn delete_label(conns: &mut [ConnectionConfig], cat: &mut Vec<Label>, name: &str) {
    cat.retain(|l| l.name != name);
    for c in conns {
        c.labels.retain(|n| n != name);
    }
}

pub fn validate_label(name: &str, cat: &[Label]) -> Result<(), String> {
    if name.trim().is_empty() {
        Err("name is required".into())
    } else if cat.iter().any(|l| l.name == name) {
        Err("name already in use".into())
    } else {
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
#[path = "config_tests.rs"]
mod tests;
