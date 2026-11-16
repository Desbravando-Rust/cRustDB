#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

fn assert_names(f: &mut ConnectionForm, role: Role, names: &[&str]) {
    // Arrange
    let mut h = Harness::new_ui(|ui| {
        show(ui, f, &[]);
    });

    // Act
    h.run();

    // Assert
    for name in names {
        h.get_by_role_and_label(role, name);
    }
}

#[test]
fn ssh_password_auth_names_password_input() {
    let mut f = ConnectionForm::new(DriverKind::Postgres, true);
    f.set_ssh(true);
    f.draft.ssh.as_mut().unwrap().auth = SshAuth::Password;

    assert_names(&mut f, Role::PasswordInput, &["SSH password"]);
}

#[test]
fn ssh_key_auth_names_key_inputs() {
    let mut f = ConnectionForm::new(DriverKind::Postgres, true);
    f.set_ssh(true);
    f.draft.ssh.as_mut().unwrap().auth = SshAuth::Key {
        key_path: PathBuf::new(),
    };

    assert_names(&mut f, Role::TextInput, &["Key file"]);
    assert_names(&mut f, Role::PasswordInput, &["Passphrase"]);
}

#[test]
fn tls_require_names_path_inputs() {
    let mut f = ConnectionForm::new(DriverKind::Postgres, true);
    f.draft.tls.mode = TlsMode::Require;

    assert_names(
        &mut f,
        Role::TextInput,
        &["CA file", "Client certificate", "Client key"],
    );
}

#[test]
fn redis_form_names_optional_inputs() {
    let mut f = ConnectionForm::new(DriverKind::Redis, true);

    assert_names(
        &mut f,
        Role::TextInput,
        &["Database index", "Key separator"],
    );
}

#[test]
fn sqlite_form_names_file_input() {
    let mut f = ConnectionForm::new(DriverKind::Sqlite, true);

    assert_names(&mut f, Role::TextInput, &["Database file"]);
}
