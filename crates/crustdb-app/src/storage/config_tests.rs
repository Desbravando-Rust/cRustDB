#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    // Arrange
    let (_t, s) = store();
    fs::write(&s.path, SPEC).unwrap();

    // Act & Assert
    let loaded = s.load().unwrap();
    assert_eq!(loaded.connections.len(), 1);
    s.save(&loaded.connections, &loaded.labels).unwrap();
    let a: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    let mut b: serde_json::Value = serde_json::from_slice(&fs::read(&s.path).unwrap()).unwrap();
    // The catalog is the only addition to the v1 document.
    assert!(b.as_object_mut().unwrap().remove("labels").is_some());
    assert_eq!(a, b);
}

#[test]
fn config_roundtrip_has_no_secrets() {
    // Arrange
    let (_t, s) = store();

    // Act
    s.save(&[sample()], &[]).unwrap();

    // Assert
    assert!(
        !fs::read_to_string(&s.path)
            .unwrap()
            .contains("\"password\"")
    );
}

#[test]
fn config_written_atomically_with_0600() {
    // Arrange
    let (t, s) = store();

    // Act
    s.save(&[sample()], &[]).unwrap();

    // Assert
    assert_eq!(
        s.path.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!t.path().join("connections.json.tmp").exists());
}

#[test]
fn unknown_version_opens_read_only() {
    // Arrange
    let (_t, s) = store();
    let v1: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    let raw = serde_json::json!({"version": 99, "connections": v1["connections"]}).to_string();
    fs::write(&s.path, &raw).unwrap();

    // Act
    let l = s.load().unwrap();

    // Assert
    assert!(l.read_only && l.warning.is_some());
    assert_eq!(l.connections.len(), 1);
    assert!(matches!(s.save(&[], &[]), Err(ConfigError::ReadOnly)));
    assert_eq!(fs::read_to_string(&s.path).unwrap(), raw);
}

#[test]
fn stale_tmp_with_open_mode_does_not_leak_into_final() {
    // Arrange
    let (t, s) = store();
    let tmp = t.path().join("connections.json.tmp");
    fs::write(&tmp, "x").unwrap();
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();

    // Act
    s.save(&[sample()], &[]).unwrap();

    // Assert
    assert_eq!(
        s.path.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn invalid_json_backed_up_and_empty() {
    // Arrange
    let (t, s) = store();
    fs::write(&s.path, "{not json").unwrap();

    // Act & Assert
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
    // Arrange
    let ok = sample();

    // Act & Assert
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
    // Arrange
    let mut c = sample();
    c.driver = DriverKind::Sqlite;
    c.host = String::new();
    c.port = 0;
    c.database = Some("/tmp/a.db".into());

    // Act & Assert
    assert!(validate(&c, &[]).is_ok());
    c.database = None;
    let errs = validate(&c, &[]).unwrap_err();
    let fields: Vec<_> = errs.iter().map(|e| e.field).collect();
    assert_eq!(fields, ["database"]);
}

#[test]
fn legacy_v1_without_labels_loads_default_catalog() {
    // Arrange
    let (t, s) = store();
    fs::write(&s.path, SPEC).unwrap();

    // Act
    let l = s.load().unwrap();

    // Assert
    let names: Vec<_> = l.labels.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["local", "dev", "staging", "production"]);
    assert!(l.warning.is_none() && !l.read_only);
    assert!(l.connections[0].labels.is_empty());
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}

#[test]
fn labels_roundtrip() {
    // Arrange
    let (_t, s) = store();
    let mut c = sample();
    c.labels = vec!["prod".into()];
    let cat = vec![Label {
        name: "prod".into(),
        color: [1, 2, 3],
    }];

    // Act
    s.save(&[c.clone()], &cat).unwrap();
    let l = s.load().unwrap();

    // Assert
    assert_eq!(l.labels, cat);
    assert_eq!(l.connections, [c]);
}

#[test]
fn empty_catalog_stays_empty() {
    // Arrange
    let (_t, s) = store();
    s.save(&[], &[]).unwrap();

    // Act
    let l = s.load().unwrap();

    // Assert
    assert!(l.labels.is_empty());
}

fn lab(n: &str) -> Label {
    Label {
        name: n.into(),
        color: [0, 0, 0],
    }
}

#[test]
fn rename_label_updates_connections() {
    // Arrange
    let mut c = sample();
    c.labels = vec!["a".into(), "b".into()];
    let mut conns = [c];
    let mut cat = [lab("a"), lab("b")];

    // Act
    rename_label(&mut conns, &mut cat, "a", "z");

    // Assert
    assert_eq!(conns[0].labels, ["z", "b"]);
    assert_eq!(cat[0].name, "z");
}

#[test]
fn delete_label_removes_from_connections() {
    // Arrange
    let mut c = sample();
    c.labels = vec!["a".into(), "b".into()];
    let mut conns = vec![c];
    let mut cat = vec![lab("a"), lab("b")];

    // Act
    delete_label(&mut conns, &mut cat, "a");

    // Assert
    assert_eq!(conns[0].labels, ["b"]);
    assert_eq!(cat, [lab("b")]);
}

#[test]
fn validate_label_rejects_empty_and_duplicate_name() {
    // Arrange
    let cat = [lab("a"), lab("b")];

    // Act & Assert
    assert!(validate_label("c", &cat).is_ok());
    assert!(validate_label("  ", &cat).is_err());
    assert!(validate_label("a", &cat).is_err());
}
