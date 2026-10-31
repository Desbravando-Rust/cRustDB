#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn only_new_forms_show_url_box() {
    // Arrange
    let cfg = ConnectionForm::new(DriverKind::Postgres, true).draft;

    // Act
    let new = ConnectionForm::new(DriverKind::Postgres, true);
    let edit = ConnectionForm::from_config(&cfg, true);

    // Assert
    assert!(new.show_url);
    assert!(!edit.show_url);
}

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
fn redis_url_import_fills_form() {
    // Arrange
    let mut f = form();

    // Act
    f.import_url("redis://h/3").unwrap();

    // Assert
    assert_eq!(f.draft.driver, DriverKind::Redis);
    assert_eq!(f.draft.port, 6379);
    assert_eq!(f.draft.database.as_deref(), Some("3"));
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

#[test]
fn import_url_sets_client_cert_paths() {
    // Arrange
    let mut f = form();

    // Act
    f.import_url("postgres://h/db?sslcert=/c/client.crt&sslkey=/c/client.key")
        .unwrap();

    // Assert
    assert_eq!(
        f.draft.tls.client_cert_path.as_deref(),
        Some(Path::new("/c/client.crt"))
    );
    assert_eq!(
        f.draft.tls.client_key_path.as_deref(),
        Some(Path::new("/c/client.key"))
    );
}

fn field_errors(f: &ConnectionForm) -> Vec<&'static str> {
    f.to_save(&[])
        .err()
        .unwrap_or_default()
        .iter()
        .map(|e| e.field)
        .collect()
}

#[test]
fn cert_without_key_is_an_error() {
    // Arrange
    let mut f = form();
    f.draft.tls.client_cert_path = Some("/etc/hostname".into());

    // Act & Assert
    assert!(field_errors(&f).contains(&"client_key"));
}

#[test]
fn key_without_cert_is_an_error() {
    // Arrange
    let mut f = form();
    f.draft.tls.client_key_path = Some("/etc/hostname".into());

    // Act & Assert
    assert!(field_errors(&f).contains(&"client_cert"));
}

#[test]
fn unreadable_client_cert_is_an_error() {
    // Arrange
    let mut f = form();
    f.draft.tls.client_cert_path = Some("/nonexistent/c.crt".into());
    f.draft.tls.client_key_path = Some("/etc/hostname".into());

    // Act & Assert
    assert_eq!(field_errors(&f), vec!["client_cert"]);
}

#[test]
fn sqlite_and_redis_clear_client_cert() {
    for driver in [DriverKind::Sqlite, DriverKind::Redis] {
        // Arrange
        let mut f = form();
        f.set_driver(driver);
        f.draft.database = (driver == DriverKind::Sqlite).then(|| "/tmp/a.db".into());
        f.draft.tls.client_cert_path = Some("/nonexistent/c.crt".into());
        f.draft.tls.client_key_path = Some("/nonexistent/c.key".into());

        // Act
        let (cfg, _) = f.to_save(&[]).unwrap();

        // Assert
        assert!(cfg.tls.client_cert_path.is_none() && cfg.tls.client_key_path.is_none());
    }
}
