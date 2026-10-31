//! Connection form view: edits the form in place, emits intents for the buttons.
use std::path::PathBuf;

use crustdb_core::driver::TlsMode;
use crustdb_core::model::DriverKind;
use eframe::egui;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

use super::Intent;
use crate::state::connection_form::{ConnectionForm, TestResult};
use crate::storage::config::{Label, SshAuth};

fn secret_edit(ui: &mut egui::Ui, v: &mut SecretString) {
    let mut s = Zeroizing::new(v.expose_secret().to_owned());
    if ui
        .add(egui::TextEdit::singleline(&mut *s).password(true))
        .changed()
    {
        *v = SecretString::from(s.as_str());
    }
}

pub(super) fn path_edit(ui: &mut egui::Ui, p: &mut Option<PathBuf>) {
    let mut s = p
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if ui.text_edit_singleline(&mut s).changed() {
        *p = (!s.is_empty()).then(|| PathBuf::from(s));
    }
}

pub(super) fn error_for(ui: &mut egui::Ui, f: &ConnectionForm, field: &str) {
    for e in f.errors.iter().filter(|e| e.field == field) {
        ui.colored_label(ui.visuals().error_fg_color, &e.message);
    }
}

/// Server fields; SQLite has none of these.
fn network_fields(ui: &mut egui::Ui, f: &mut ConnectionForm) {
    ui.label("Host");
    ui.text_edit_singleline(&mut f.draft.host);
    ui.end_row();
    error_for(ui, f, "host");
    ui.end_row();

    ui.label("Port");
    ui.add(egui::DragValue::new(&mut f.draft.port));
    ui.end_row();
    error_for(ui, f, "port");
    ui.end_row();

    ui.label("User");
    ui.text_edit_singleline(&mut f.draft.user);
    ui.end_row();

    ui.label("Password");
    secret_edit(ui, &mut f.password);
    ui.end_row();

    ui.label("");
    ui.add_enabled(
        f.keyring_available,
        egui::Checkbox::new(&mut f.save_in_keyring, "Save in keyring"),
    );
    ui.end_row();
    if let Some(h) = f.keyring_hint() {
        ui.label("");
        ui.weak(h);
        ui.end_row();
    }

    if f.draft.driver == DriverKind::Redis {
        super::connection_form_redis::fields(ui, f);
    } else {
        ui.label("Database");
        let mut db = f.draft.database.clone().unwrap_or_default();
        if ui.text_edit_singleline(&mut db).changed() {
            f.draft.database = (!db.is_empty()).then_some(db);
        }
        ui.end_row();
    }

    let tls_label = ui.label("TLS mode").on_hover_text(
        "Disable: no encryption.\n\
         Require: encrypt, don't verify the server certificate.\n\
         VerifyCa: encrypt and verify the certificate is signed by a trusted CA.\n\
         VerifyFull: VerifyCa plus check the host name matches the certificate.",
    );
    let tls = egui::ComboBox::from_id_salt("tls")
        .selected_text(format!("{:?}", f.draft.tls.mode))
        .show_ui(ui, |ui| {
            for (m, l) in [
                (TlsMode::Disable, "Disable"),
                (TlsMode::Require, "Require"),
                (TlsMode::VerifyCa, "VerifyCa"),
                (TlsMode::VerifyFull, "VerifyFull"),
            ] {
                ui.selectable_value(&mut f.draft.tls.mode, m, l);
            }
        });
    tls.response.labelled_by(tls_label.id);
    ui.end_row();
    for n in f.tls_notes() {
        ui.label("");
        ui.colored_label(ui.visuals().warn_fg_color, n);
        ui.end_row();
    }

    ui.label("CA file").on_hover_text(
        "PEM file with the CA certificate used to verify the server (VerifyCa/VerifyFull). Empty = system roots.",
    );
    path_edit(ui, &mut f.draft.tls.ca_path);
    ui.end_row();
    ui.label("");
    ui.weak("Empty = system roots");
    ui.end_row();
    error_for(ui, f, "ca_path");
    ui.end_row();
    super::connection_form_mtls::fields(ui, f);
}

pub fn show(ui: &mut egui::Ui, f: &mut ConnectionForm, labels: &[Label]) -> Vec<Intent> {
    let mut out = vec![];
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("Connection");
            if f.show_url {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut *f.url_input);
                    if ui.button("Import").clicked() {
                        out.push(Intent::FormImportUrl);
                    }
                });
                ui.weak("e.g. postgres://user@localhost:5432/mydb");
                if let Some(e) = &f.url_error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            for w in &f.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, w);
            }
            egui::Grid::new("connection_form")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut f.draft.name);
                    ui.end_row();
                    error_for(ui, f, "name");
                    ui.end_row();

                    let driver_label = ui.label("Driver");
                    let mut driver = f.draft.driver;
                    let driver_combo = egui::ComboBox::from_id_salt("driver")
                        .selected_text(match driver {
                            DriverKind::Postgres => "PostgreSQL",
                            DriverKind::Mysql => "MySQL/MariaDB",
                            DriverKind::Sqlite => "SQLite",
                            DriverKind::Redis => "Redis",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut driver, DriverKind::Postgres, "PostgreSQL");
                            ui.selectable_value(&mut driver, DriverKind::Mysql, "MySQL/MariaDB");
                            ui.selectable_value(&mut driver, DriverKind::Sqlite, "SQLite");
                            ui.selectable_value(&mut driver, DriverKind::Redis, "Redis");
                        });
                    driver_combo.response.labelled_by(driver_label.id);
                    if driver != f.draft.driver {
                        f.set_driver(driver);
                    }
                    ui.end_row();

                    if f.draft.driver == DriverKind::Sqlite {
                        // A local file: no server, credentials, TLS or tunnel.
                        ui.label("Database file");
                        ui.horizontal(|ui| {
                            let mut p = f.draft.database.as_ref().map(PathBuf::from);
                            path_edit(ui, &mut p);
                            f.draft.database = p.map(|p| p.to_string_lossy().into_owned());
                            if ui.button("Browse…").clicked() {
                                out.push(Intent::FormBrowseSqlite);
                            }
                        });
                        ui.end_row();
                        error_for(ui, f, "database");
                        ui.end_row();
                    } else {
                        network_fields(ui, f);
                    }

                    ui.label("Page size")
                        .on_hover_text("Rows fetched per page when browsing table data.");
                    ui.add(egui::DragValue::new(&mut f.draft.page_size));
                    ui.end_row();
                    error_for(ui, f, "page_size");
                    ui.end_row();
                    ui.label("Editor row limit")
                        .on_hover_text("Rows fetched per batch for SQL editor queries; use Load more to fetch the next batch.");
                    ui.add(egui::DragValue::new(&mut f.draft.editor_row_limit));
                    ui.end_row();
                    error_for(ui, f, "editor_row_limit");
                    ui.end_row();
                    ui.label("Safety threshold").on_hover_text(
                        "Estimated affected rows above which DML, TRUNCATE or DROP TABLE require password confirmation.",
                    );
                    ui.add(egui::DragValue::new(&mut f.draft.safety_threshold));
                    ui.end_row();
                    error_for(ui, f, "safety_threshold");
                    ui.end_row();
                });

            let sqlite = f.draft.driver == DriverKind::Sqlite;
            let mut ssh_on = f.draft.ssh.is_some();
            if !sqlite
                && ui
                    .checkbox(&mut ssh_on, "SSH tunnel")
                    .on_hover_text(
                        "Connect through an SSH server (bastion) that forwards traffic to the database.",
                    )
                    .changed()
            {
                f.set_ssh(ssh_on);
            }
            if let Some(ssh) = f.draft.ssh.as_mut().filter(|_| !sqlite) {
                egui::Grid::new("ssh_form").num_columns(2).show(ui, |ui| {
                    ui.label("SSH host");
                    ui.text_edit_singleline(&mut ssh.host);
                    ui.end_row();
                    ui.label("SSH port");
                    ui.add(egui::DragValue::new(&mut ssh.port));
                    ui.end_row();
                    ui.label("SSH user");
                    ui.text_edit_singleline(&mut ssh.user);
                    ui.end_row();
                    let auth_label = ui.label("Auth");
                    let cur = match ssh.auth {
                        SshAuth::Password => "Password",
                        SshAuth::Key { .. } => "Key file",
                        SshAuth::Agent => "Agent",
                    };
                    let auth_combo = egui::ComboBox::from_id_salt("ssh_auth")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(cur == "Password", "Password").clicked() {
                                ssh.auth = SshAuth::Password;
                            }
                            if ui.selectable_label(cur == "Key file", "Key file").clicked()
                                && !matches!(ssh.auth, SshAuth::Key { .. })
                            {
                                ssh.auth = SshAuth::Key {
                                    key_path: PathBuf::new(),
                                };
                            }
                            if ui.selectable_label(cur == "Agent", "Agent").clicked() {
                                ssh.auth = SshAuth::Agent;
                            }
                        });
                    auth_combo.response.labelled_by(auth_label.id);
                    ui.end_row();
                    match &mut ssh.auth {
                        SshAuth::Password => {
                            ui.label("SSH password");
                            secret_edit(ui, &mut f.ssh_password);
                            ui.end_row();
                        }
                        SshAuth::Key { key_path } => {
                            ui.label("Key file");
                            let mut p = Some(std::mem::take(key_path));
                            path_edit(ui, &mut p);
                            *key_path = p.unwrap_or_default();
                            ui.end_row();
                            ui.label("");
                            ui.weak("ed25519 or ecdsa keys only (RSA is not supported)");
                            ui.end_row();
                            ui.label("Passphrase");
                            secret_edit(ui, &mut f.ssh_passphrase);
                            ui.end_row();
                        }
                        SshAuth::Agent => {}
                    }
                });
                for field in ["ssh_host", "ssh_port", "ssh_user", "ssh_key"] {
                    error_for(ui, f, field);
                }
            }

            ui.separator();
            super::labels::picker(ui, &mut f.draft.label, labels);
            match &f.test {
                TestResult::Idle => {}
                TestResult::Running(_) => {
                    ui.label("Testing...");
                }
                TestResult::Ok(v) => {
                    ui.label(format!("Connection OK ({v})"));
                }
                TestResult::Failed(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            ui.horizontal(|ui| {
                let idle = !matches!(f.test, TestResult::Running(_));
                if ui.add_enabled(idle, egui::Button::new("Test")).clicked() {
                    out.push(Intent::FormTest);
                }
                if ui.button("Save").clicked() {
                    out.push(Intent::FormSave);
                }
                if ui.button("Connect").clicked() {
                    out.push(Intent::FormConnect);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Intent::FormCancel);
                }
            });
        });
    out
}

#[cfg(test)]
#[path = "connection_form_tests.rs"]
mod tests;
