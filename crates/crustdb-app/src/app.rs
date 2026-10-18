use eframe::egui;
use std::sync::mpsc::Receiver;

use crustdb_core::model::ConnectionId;

use crate::net::ssh::SshSecrets;
use crate::runtime::{Command, Event, ResolvedSecrets, Runtime};
use crate::state::AppState;
use crate::storage::config::{ConfigStore, LoadedConfig};
use crate::storage::secrets::{SecretKind, SecretStore, account};
use crate::ui::{self, Intent};

pub struct CrustApp {
    state: AppState,
    runtime: Option<Runtime>,
    events: Receiver<Event>,
    store: ConfigStore,
    secrets: Box<dyn SecretStore>,
}

impl CrustApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut config: LoadedConfig,
        store: ConfigStore,
        secrets: Box<dyn SecretStore>,
        drivers: crate::runtime::DriverRegistry,
    ) -> Self {
        ui::theme::install_theme(&cc.egui_ctx);
        if !secrets.available() {
            config.warning.get_or_insert_with(|| {
                "OS keyring unavailable: passwords cannot be saved".to_owned()
            });
        }
        let ctx = cc.egui_ctx.clone();
        let (runtime, events) =
            Runtime::start(std::sync::Arc::new(move || ctx.request_repaint()), drivers);
        Self {
            state: AppState::new(config),
            runtime: Some(runtime),
            events,
            store,
            secrets,
        }
    }

    fn secret(&self, id: ConnectionId, kind: SecretKind) -> Option<secrecy::SecretString> {
        // Errors carry no secret material, but only the fact is logged.
        self.secrets
            .get(&account(id, kind))
            .inspect_err(|e| tracing::warn!("keyring read failed: {e}"))
            .ok()
            .flatten()
    }

    fn handle(&mut self, intent: Intent) {
        match intent {
            Intent::Connect(id) => {
                let Some(config) = self
                    .state
                    .config
                    .connections
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                else {
                    return;
                };
                let secrets = ResolvedSecrets {
                    db: self.secret(id, SecretKind::Db),
                    ssh: SshSecrets {
                        password: self.secret(id, SecretKind::SshPassword),
                        passphrase: self.secret(id, SecretKind::SshPassphrase),
                    },
                };
                let ws = self.state.open_workspace(config.clone());
                if let Some(rt) = &self.runtime {
                    rt.send(Command::Connect {
                        ws,
                        config,
                        secrets,
                    });
                }
            }
            Intent::DeleteConnection(id) => self.state.request_delete(id),
            Intent::CancelDialog => {
                self.state.dialogs.pop();
            }
            Intent::ConfirmDelete => self.confirm_delete(),
            // Handled by the connection form (next task).
            Intent::NewConnection | Intent::EditConnection(_) | Intent::ImportUrl => {}
        }
    }

    fn confirm_delete(&mut self) {
        let store = &self.store;
        match self.state.confirm_delete(|rest| store.save(rest)) {
            Ok(Some(id)) => {
                for kind in [
                    SecretKind::Db,
                    SecretKind::SshPassword,
                    SecretKind::SshPassphrase,
                ] {
                    if let Err(e) = self.secrets.delete(&account(id, kind)) {
                        tracing::warn!("keyring delete failed: {e}");
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                self.state.config.warning = Some(format!("could not save connections: {e}"));
            }
        }
    }
}

impl eframe::App for CrustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(e) = self.events.try_recv() {
            self.state.apply(e);
        }
        let mut intents = vec![];
        egui::CentralPanel::default().show(ui, |ui| {
            intents.extend(ui::home::show(ui, &self.state));
        });
        intents.extend(ui::home::dialogs(ui.ctx(), &self.state));
        for i in intents {
            self.handle(i);
        }
    }

    fn on_exit(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown();
        }
    }
}
