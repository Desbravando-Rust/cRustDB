//! Connection form state: draft config, secret buffers, validation and URL import.
use std::path::Path;

use crustdb_core::driver::{TlsConfig, TlsMode};
use crustdb_core::model::{ConnectionId, DriverKind};
use crustdb_core::url::{ParsedUrl, UrlError, UrlWarning, parse_connection_url};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

use crate::net::ssh::SshSecrets;
use crate::runtime::{Command, Event, ResolvedSecrets, WorkspaceId};
use crate::storage::config::{ConnectionConfig, FieldError, SshAuth, SshConfig, validate};
use crate::storage::secrets::SecretKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestResult {
    Idle,
    Running(WorkspaceId),
    Ok(String),
    Failed(String),
}

pub type Saved = (ConnectionConfig, Vec<(SecretKind, SecretString)>);

pub struct ConnectionForm {
    pub draft: ConnectionConfig,
    pub password: SecretString,
    pub save_in_keyring: bool,
    pub ssh_password: SecretString,
    pub ssh_passphrase: SecretString,
    pub errors: Vec<FieldError>,
    pub warnings: Vec<String>,
    pub keyring_available: bool,
    pub test: TestResult,
    /// Import-by-URL box; only shown when opened through "Import URL".
    pub show_url: bool,
    pub url_input: Zeroizing<String>,
    pub url_error: Option<String>,
}

pub fn default_port(driver: DriverKind) -> u16 {
    match driver {
        DriverKind::Postgres => 5432,
        DriverKind::Mysql => 3306,
        DriverKind::Sqlite => 0,
    }
}

fn default_config(driver: DriverKind) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(uuid::Uuid::new_v4()),
        name: String::new(),
        driver,
        host: String::new(),
        port: default_port(driver),
        user: String::new(),
        database: None,
        save_password: false,
        tls: TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_path: None,
        },
        ssh: None,
        page_size: 300,
        editor_row_limit: 1000,
        safety_threshold: 1000,
    }
}

fn empty() -> SecretString {
    SecretString::from(String::new())
}

fn is_empty(s: &SecretString) -> bool {
    s.expose_secret().is_empty()
}

fn readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok_and(|f| f.metadata().is_ok_and(|m| m.is_file()))
}

impl ConnectionForm {
    pub fn new(driver: DriverKind, keyring_available: bool) -> Self {
        Self::from_config(&default_config(driver), keyring_available)
    }

    pub fn from_config(c: &ConnectionConfig, keyring_available: bool) -> Self {
        Self {
            draft: c.clone(),
            password: empty(),
            save_in_keyring: keyring_available && c.save_password,
            ssh_password: empty(),
            ssh_passphrase: empty(),
            errors: vec![],
            warnings: vec![],
            keyring_available,
            test: TestResult::Idle,
            show_url: false,
            url_input: Zeroizing::new(String::new()),
            url_error: None,
        }
    }

    /// Switching driver moves the port only while it still holds the old default.
    pub fn set_driver(&mut self, driver: DriverKind) {
        if self.draft.port == default_port(self.draft.driver) {
            self.draft.port = default_port(driver);
        }
        self.draft.driver = driver;
    }

    pub fn import_url(&mut self, s: &str) -> Result<(), UrlError> {
        let ParsedUrl {
            driver,
            host,
            port,
            user,
            password,
            database,
            tls_mode,
            warnings,
        } = parse_connection_url(s.trim())?;
        self.set_driver(driver);
        if self.draft.name.trim().is_empty() {
            // SQLite has no host: name it after the file.
            let file = database.as_deref().and_then(|d| Path::new(d).file_name());
            self.draft.name = match file {
                Some(f) if host.is_empty() => f.to_string_lossy().into_owned(),
                _ => host.clone(),
            };
        }
        self.draft.host = host;
        self.draft.port = port.unwrap_or_else(|| default_port(driver));
        self.draft.user = user.unwrap_or_default();
        self.draft.database = database;
        if let Some(m) = tls_mode {
            self.draft.tls.mode = m;
        }
        if let Some(p) = password {
            self.password = p;
        }
        self.warnings = warnings
            .into_iter()
            .map(|UrlWarning::DowngradedToRequire(v)| {
                format!("sslmode \"{v}\" is not supported; using require")
            })
            .collect();
        Ok(())
    }

    pub fn tls_downgraded(&self) -> bool {
        self.draft.tls.mode != TlsMode::VerifyFull
    }

    pub fn tls_notes(&self) -> Vec<&'static str> {
        let mut n = vec![];
        if self.tls_downgraded() {
            n.push("TLS is below verify-full: the server identity is not fully verified.");
        }
        if self.draft.driver == DriverKind::Mysql {
            if self.draft.tls.mode == TlsMode::VerifyCa {
                n.push("MySQL: verify-ca behaves as verify-full (fail-closed).");
            }
            n.push("MariaDB 10.0 does not support TLS: use an SSH tunnel or disable TLS.");
        }
        n
    }

    pub fn keyring_hint(&self) -> Option<&'static str> {
        (!self.keyring_available).then_some("OS keyring unavailable: passwords cannot be saved")
    }

    fn saves_secrets(&self) -> bool {
        self.save_in_keyring && self.keyring_available
    }

    fn validated(&self, others: &[ConnectionConfig]) -> Result<ConnectionConfig, Vec<FieldError>> {
        let mut c = self.draft.clone();
        c.save_password = self.saves_secrets();
        c.database = c.database.filter(|d| !d.is_empty());
        if c.driver == DriverKind::Sqlite {
            // A local file: no tunnel, CA or saved password.
            c.ssh = None;
            c.tls.ca_path = None;
            c.save_password = false;
        }
        let mut errs = validate(&c, others).err().unwrap_or_default();
        let mut bad = |field, message: &str| {
            errs.push(FieldError {
                field,
                message: message.into(),
            })
        };
        if let Some(ca) = &c.tls.ca_path
            && !readable(ca)
        {
            bad("ca_path", "CA file not found or unreadable");
        }
        if let Some(ssh) = &c.ssh {
            if ssh.host.trim().is_empty() {
                bad("ssh_host", "SSH host is required");
            }
            if ssh.port == 0 {
                bad("ssh_port", "SSH port must be 1-65535");
            }
            if ssh.user.trim().is_empty() {
                bad("ssh_user", "SSH user is required");
            }
            if let SshAuth::Key { key_path } = &ssh.auth
                && !readable(key_path)
            {
                bad("ssh_key", "key file not found or unreadable");
            }
        }
        if errs.is_empty() { Ok(c) } else { Err(errs) }
    }

    pub fn to_save(&self, others: &[ConnectionConfig]) -> Result<Saved, Vec<FieldError>> {
        let c = self.validated(others)?;
        let mut secrets = vec![];
        if c.save_password {
            let ssh_auth = c.ssh.as_ref().map(|s| &s.auth);
            for (kind, v, wanted) in [
                (SecretKind::Db, &self.password, true),
                (
                    SecretKind::SshPassword,
                    &self.ssh_password,
                    matches!(ssh_auth, Some(SshAuth::Password)),
                ),
                (
                    SecretKind::SshPassphrase,
                    &self.ssh_passphrase,
                    matches!(ssh_auth, Some(SshAuth::Key { .. })),
                ),
            ] {
                if wanted && !is_empty(v) {
                    secrets.push((kind, v.clone()));
                }
            }
        }
        Ok((c, secrets))
    }

    /// Secrets typed into the form (empty ones omitted); the caller fills the rest from the keyring.
    pub fn typed_secrets(&self) -> ResolvedSecrets {
        let some = |v: &SecretString| (!is_empty(v)).then(|| v.clone());
        ResolvedSecrets {
            db: some(&self.password),
            ssh: SshSecrets {
                password: some(&self.ssh_password),
                passphrase: some(&self.ssh_passphrase),
            },
        }
    }

    pub fn start_test(
        &mut self,
        ws: WorkspaceId,
        others: &[ConnectionConfig],
    ) -> Result<Command, Vec<FieldError>> {
        let config = self
            .validated(others)
            .inspect_err(|e| self.errors.clone_from(e))?;
        self.errors.clear();
        self.test = TestResult::Running(ws);
        Ok(Command::Connect {
            ws,
            config,
            secrets: self.typed_secrets(),
        })
    }

    /// Records the outcome of this form's test connection; true if `e` was it.
    pub fn on_event(&mut self, e: &Event) -> bool {
        let TestResult::Running(id) = self.test else {
            return false;
        };
        match e {
            Event::Connected { ws, server } if *ws == id => {
                self.test = TestResult::Ok(server.raw.clone());
            }
            Event::ConnectFailed { ws, error } if *ws == id => {
                self.test = TestResult::Failed(error.clone());
            }
            _ => return false,
        }
        true
    }

    /// Workspace of an in-flight test, if any; the caller must disconnect it when dropping the form.
    pub fn take_pending_test(&mut self) -> Option<WorkspaceId> {
        match std::mem::replace(&mut self.test, TestResult::Idle) {
            TestResult::Running(ws) => Some(ws),
            other => {
                self.test = other;
                None
            }
        }
    }

    pub fn set_ssh(&mut self, on: bool) {
        self.draft.ssh = on.then(|| SshConfig {
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: SshAuth::Agent,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn form() -> ConnectionForm {
        let mut f = ConnectionForm::new(DriverKind::Postgres, true);
        f.draft.name = "n".into();
        f.draft.host = "h".into();
        f
    }

    #[test]
    fn default_ports_follow_driver() {
        // Arrange
        let mut f = form();

        // Act & Assert
        assert_eq!(f.draft.port, 5432);
        f.set_driver(DriverKind::Mysql);
        assert_eq!(f.draft.port, 3306);
        f.draft.port = 4000;
        f.set_driver(DriverKind::Postgres);
        assert_eq!(f.draft.port, 4000);
        let d = ConnectionForm::new(DriverKind::Mysql, true).draft;
        assert_eq!(
            (d.page_size, d.editor_row_limit, d.safety_threshold),
            (300, 1000, 1000)
        );
        assert_eq!(d.tls.mode, TlsMode::VerifyFull);
    }

    #[test]
    fn import_url_fills_fields_and_keeps_password_out_of_config() {
        // Arrange
        let mut f = form();

        // Act & Assert
        f.import_url("mysql://bob:s3cret@db.example.com/shop?ssl-mode=preferred")
            .unwrap();
        assert_eq!(f.draft.driver, DriverKind::Mysql);
        assert_eq!(f.draft.host, "db.example.com");
        assert_eq!(f.draft.port, 3306);
        assert_eq!(f.draft.user, "bob");
        assert_eq!(f.draft.database.as_deref(), Some("shop"));
        assert_eq!(f.draft.tls.mode, TlsMode::Require);
        assert_eq!(f.password.expose_secret(), "s3cret");
        assert_eq!(f.warnings.len(), 1);
        f.save_in_keyring = false;
        let (cfg, secrets) = f.to_save(&[]).unwrap();
        assert!(!serde_json::to_string(&cfg).unwrap().contains("s3cret"));
        assert!(secrets.is_empty());
        assert!(f.import_url("ftp://x").is_err());
    }

    #[test]
    fn tls_below_verify_full_shows_warning() {
        // Arrange
        let mut f = form();

        // Act & Assert
        assert!(!f.tls_downgraded());
        assert!(f.tls_notes().is_empty());
        f.draft.tls.mode = TlsMode::Require;
        assert!(f.tls_downgraded());
        assert!(!f.tls_notes().is_empty());
        f.set_driver(DriverKind::Mysql);
        f.draft.tls.mode = TlsMode::VerifyCa;
        assert!(f.tls_notes().iter().any(|n| n.contains("verify-full")));
    }

    #[test]
    fn save_without_keyring_checkbox_stores_no_secret() {
        // Arrange
        let mut f = form();
        f.password = "pw".to_owned().into();
        f.save_in_keyring = false;

        // Act & Assert
        let (cfg, secrets) = f.to_save(&[]).unwrap();
        assert!(!cfg.save_password && secrets.is_empty());
        f.save_in_keyring = true;
        let (cfg, secrets) = f.to_save(&[]).unwrap();
        assert!(cfg.save_password);
        assert_eq!(secrets.len(), 1);
        assert!(matches!(secrets[0].0, SecretKind::Db));
    }

    #[test]
    fn keyring_unavailable_disables_checkbox_with_hint() {
        // Arrange
        let mut f = ConnectionForm::new(DriverKind::Postgres, false);
        f.draft.name = "n".into();
        f.draft.host = "h".into();

        // Act & Assert
        assert!(!f.save_in_keyring);
        assert!(f.keyring_hint().is_some());
        f.password = "pw".to_owned().into();
        f.save_in_keyring = true;
        let (cfg, secrets) = f.to_save(&[]).unwrap();
        assert!(!cfg.save_password && secrets.is_empty());
        assert!(form().keyring_hint().is_none());
    }

    #[test]
    fn test_button_sends_connect_with_ephemeral_workspace() {
        // Arrange
        let mut f = form();
        f.password = "pw".to_owned().into();
        let cmd = f.start_test(WorkspaceId(7), &[]).unwrap();

        // Act & Assert
        let Command::Connect {
            ws,
            config,
            secrets,
        } = cmd
        else {
            panic!("expected Connect");
        };
        assert_eq!(ws, WorkspaceId(7));
        assert_eq!(config.host, "h");
        assert_eq!(secrets.db.unwrap().expose_secret(), "pw");
        assert_eq!(f.test, TestResult::Running(WorkspaceId(7)));
        // events for other workspaces are not ours
        assert!(!f.on_event(&Event::ConnectFailed {
            ws: WorkspaceId(8),
            error: "x".into()
        }));
        assert!(f.on_event(&Event::ConnectFailed {
            ws: WorkspaceId(7),
            error: "refused".into()
        }));
        assert_eq!(f.test, TestResult::Failed("refused".into()));
        f.draft.host.clear();
        assert!(f.start_test(WorkspaceId(9), &[]).is_err());
        assert_eq!(f.test, TestResult::Failed("refused".into()));
    }

    #[test]
    fn closing_form_mid_test_yields_workspace_to_disconnect() {
        // Arrange
        let mut f = form();

        // Act & Assert
        assert_eq!(f.take_pending_test(), None);
        f.start_test(WorkspaceId(5), &[]).unwrap();
        assert_eq!(f.take_pending_test(), Some(WorkspaceId(5)));
        assert_eq!(f.take_pending_test(), None);
        f.test = TestResult::Ok("v".into());
        assert_eq!(f.take_pending_test(), None);
        assert_eq!(f.test, TestResult::Ok("v".into()));
    }

    #[test]
    fn sqlite_form_drops_network_fields() {
        // Arrange
        let mut f = form();
        f.set_ssh(true);
        f.draft.tls.ca_path = Some("/nonexistent/ca.pem".into());
        f.password = "pw".to_owned().into();
        f.save_in_keyring = true;
        f.set_driver(DriverKind::Sqlite);
        f.draft.database = Some("/tmp/a.db".into());

        // Act
        let (cfg, secrets) = f.to_save(&[]).unwrap();

        // Assert
        assert!(cfg.ssh.is_none() && cfg.tls.ca_path.is_none());
        assert!(!cfg.save_password && secrets.is_empty());
    }

    #[test]
    fn import_sqlite_url_names_from_file() {
        // Arrange
        let mut f = ConnectionForm::new(DriverKind::Postgres, true);

        // Act
        f.import_url("sqlite:///tmp/a.db").unwrap();

        // Assert
        assert_eq!(f.draft.driver, DriverKind::Sqlite);
        assert_eq!(f.draft.database.as_deref(), Some("/tmp/a.db"));
        assert_eq!(f.draft.name, "a.db");
    }
}
