//! Connection form view: edits the form in place, emits intents for the buttons.
use std::path::PathBuf;

use crustdb_core::driver::TlsMode;
use crustdb_core::model::DriverKind;
use eframe::egui;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

use super::Intent;
use crate::state::connection_form::{ConnectionForm, TestResult};
use crate::storage::config::SshAuth;

fn secret_edit(ui: &mut egui::Ui, v: &mut SecretString) {
    let mut s = Zeroizing::new(v.expose_secret().to_owned());
    if ui
        .add(egui::TextEdit::singleline(&mut *s).password(true))
        .changed()
    {
        *v = SecretString::from(s.as_str());
    }
}

fn path_edit(ui: &mut egui::Ui, p: &mut Option<PathBuf>) {
    let mut s = p
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if ui.text_edit_singleline(&mut s).changed() {
        *p = (!s.is_empty()).then(|| PathBuf::from(s));
    }
}

fn error_for(ui: &mut egui::Ui, f: &ConnectionForm, field: &str) {
    for e in f.errors.iter().filter(|e| e.field == field) {
        ui.colored_label(ui.visuals().error_fg_color, &e.message);
    }
}

pub fn show(ui: &mut egui::Ui, f: &mut ConnectionForm) -> Vec<Intent> {
    let mut out = vec![];
    ui.heading("Connection");
    if f.show_url {
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut *f.url_input);
            if ui.button("Import").clicked() {
                out.push(Intent::FormImportUrl);
            }
        });
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

            ui.label("Driver");
            let mut driver = f.draft.driver;
            egui::ComboBox::from_id_salt("driver")
                .selected_text(match driver {
                    DriverKind::Postgres => "PostgreSQL",
                    DriverKind::Mysql => "MySQL/MariaDB",
                    DriverKind::Sqlite => "SQLite",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut driver, DriverKind::Postgres, "PostgreSQL");
                    ui.selectable_value(&mut driver, DriverKind::Mysql, "MySQL/MariaDB");
                });
            if driver != f.draft.driver {
                f.set_driver(driver);
            }
            ui.end_row();

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

            ui.label("Database");
            let mut db = f.draft.database.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut db).changed() {
                f.draft.database = (!db.is_empty()).then_some(db);
            }
            ui.end_row();

            ui.label("TLS mode");
            egui::ComboBox::from_id_salt("tls")
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
            ui.end_row();
            for n in f.tls_notes() {
                ui.label("");
                ui.colored_label(ui.visuals().warn_fg_color, n);
                ui.end_row();
            }

            ui.label("CA file");
            path_edit(ui, &mut f.draft.tls.ca_path);
            ui.end_row();
            ui.label("");
            ui.weak("Empty = system roots");
            ui.end_row();
            error_for(ui, f, "ca_path");
            ui.end_row();

            ui.label("Page size");
            ui.add(egui::DragValue::new(&mut f.draft.page_size));
            ui.end_row();
            error_for(ui, f, "page_size");
            ui.end_row();
            ui.label("Editor row limit");
            ui.add(egui::DragValue::new(&mut f.draft.editor_row_limit));
            ui.end_row();
            error_for(ui, f, "editor_row_limit");
            ui.end_row();
            ui.label("Safety threshold");
            ui.add(egui::DragValue::new(&mut f.draft.safety_threshold));
            ui.end_row();
            error_for(ui, f, "safety_threshold");
            ui.end_row();
        });

    let mut ssh_on = f.draft.ssh.is_some();
    if ui.checkbox(&mut ssh_on, "SSH tunnel").changed() {
        f.set_ssh(ssh_on);
    }
    if let Some(ssh) = &mut f.draft.ssh {
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
            ui.label("Auth");
            let cur = match ssh.auth {
                SshAuth::Password => "Password",
                SshAuth::Key { .. } => "Key file",
                SshAuth::Agent => "Agent",
            };
            egui::ComboBox::from_id_salt("ssh_auth")
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
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn form_shows_keyring_hint_and_buttons() {
        let mut f = ConnectionForm::new(DriverKind::Postgres, false);
        f.set_ssh(true);
        let mut h = Harness::builder()
            .with_accessibility_check(false)
            .build_ui(|ui| {
                show(ui, &mut f);
            });
        h.run();
        h.get_by_label("Save");
        h.get_by_label("Test");
        h.get_by_label("OS keyring unavailable: passwords cannot be saved");
    }
}
