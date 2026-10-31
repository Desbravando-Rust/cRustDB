//! Modal dialogs for the top of the stack; they only draw and return intents.
use crustdb_core::safety::{Evaluation, FlagReason, Verdict};
use eframe::egui;

use super::Intent;
use crate::runtime::{RequestId, TxOp};
use crate::state::dialogs::{MAX_PASSWORD_ATTEMPTS, PasswordInput};
use crate::state::{Dialog, DialogStack};

/// One line per flagged statement: position, reason and, when known, the row estimate.
pub fn flag_rows(ev: &Evaluation) -> Vec<String> {
    ev.flags
        .iter()
        .map(|f| {
            let why = match f.reason {
                FlagReason::NoWhere => "no WHERE clause".to_owned(),
                FlagReason::MultiRow(n) => format!("affects about {n} rows"),
                FlagReason::OverThreshold(n) => format!("about {n} rows, over the threshold"),
                FlagReason::UnknownEstimate => "impact could not be estimated".to_owned(),
                FlagReason::Destructive(k) => format!("destructive statement ({k:?})"),
                FlagReason::Unrecognized => "unrecognized statement".to_owned(),
            };
            format!("Statement {}: {why}", f.index + 1)
        })
        .collect()
}

fn password_id(exec: RequestId) -> egui::Id {
    egui::Id::unique(("safety_password", exec.0))
}

fn password_field(ui: &mut egui::Ui, id: egui::Id, input: &mut PasswordInput, attempts: u8) {
    ui.label("Type the password (or the database name if none) to continue.");
    ui.add(
        egui::TextEdit::singleline(&mut *input.0)
            .id(id)
            .password(true),
    );
    if attempts > 0 {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Wrong password ({attempts}/{MAX_PASSWORD_ATTEMPTS})"),
        );
    }
}

pub fn show(ctx: &egui::Context, dialogs: &mut DialogStack) -> Vec<Intent> {
    let mut out = vec![];
    let Some(top) = dialogs.top_mut() else {
        return out;
    };
    let pw_id = match top {
        Dialog::Safety { exec, .. } => Some(password_id(*exec)),
        Dialog::StoreSafety { req, .. } => Some(password_id(*req)),
        _ => None,
    };
    let title = match top {
        Dialog::Safety { .. } | Dialog::StoreSafety { .. } => "Confirm execution",
        Dialog::CloseWithTx { .. } => "Open transaction",
        Dialog::DirtyFile { .. } => "Unsaved changes",
        Dialog::HostKey { .. } => "Unknown SSH host key",
        Dialog::DeleteConnection { .. } => "Delete connection",
        Dialog::Disconnect { .. } => "Disconnect",
    };
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| match top {
            Dialog::Safety {
                exec,
                evaluation,
                input,
                attempts,
                ..
            } => {
                for row in flag_rows(evaluation) {
                    ui.label(row);
                }
                if evaluation.verdict == Verdict::Password {
                    password_field(ui, password_id(*exec), input, *attempts);
                }
                ui.horizontal(|ui| {
                    if ui.button("Execute").clicked() {
                        out.push(Intent::ConfirmSafety);
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::StoreSafety {
                req,
                verdict,
                reason,
                input,
                attempts,
                ..
            } => {
                ui.label(reason.as_str());
                if *verdict == Verdict::Password {
                    password_field(ui, password_id(*req), input, *attempts);
                }
                ui.horizontal(|ui| {
                    if ui.button("Execute").clicked() {
                        out.push(Intent::ConfirmSafety);
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::CloseWithTx { .. } => {
                ui.label("This tab has an open transaction.");
                ui.horizontal(|ui| {
                    if ui.button("Commit and close").clicked() {
                        out.push(Intent::CloseResolve(TxOp::Commit));
                    }
                    if ui.button("Rollback and close").clicked() {
                        out.push(Intent::CloseResolve(TxOp::Rollback));
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::DirtyFile { .. } => {
                ui.label("The file has unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        out.push(Intent::SaveFile);
                    }
                    if ui.button("Discard").clicked() {
                        out.push(Intent::DiscardFile);
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::HostKey {
                host, fingerprint, ..
            } => {
                ui.label(format!("The authenticity of {host} cannot be established."));
                ui.monospace(fingerprint.as_str());
                ui.horizontal(|ui| {
                    if ui.button("Accept & save").clicked() {
                        out.push(Intent::AcceptHostKey);
                    }
                    if ui.button("Reject").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::Disconnect {
                has_tx,
                pending_rows,
                ..
            } => {
                if *has_tx {
                    ui.label("This workspace has an open transaction.");
                }
                if *pending_rows > 0 {
                    ui.label(format!(
                        "{pending_rows} pending row edit(s) will be discarded."
                    ));
                }
                ui.horizontal(|ui| {
                    if *has_tx && ui.button("Commit and disconnect").clicked() {
                        out.push(Intent::DisconnectResolve(TxOp::Commit));
                    }
                    if ui.button("Rollback and disconnect").clicked() {
                        out.push(Intent::DisconnectResolve(TxOp::Rollback));
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
            Dialog::DeleteConnection { name, .. } => {
                ui.label(format!("Delete \"{name}\"? This cannot be undone."));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        out.push(Intent::ConfirmDelete);
                    }
                    if ui.button("Cancel").clicked() {
                        out.push(Intent::CancelDialog);
                    }
                });
            }
        });
    // The text editor's undo history holds typed passwords in clear; drop it on every action.
    if let (Some(id), false) = (pw_id, out.is_empty())
        && let Some(mut st) = egui::text_edit::TextEditState::load(ctx, id)
    {
        st.clear_undoer();
        st.store(ctx, id);
    }
    out
}
