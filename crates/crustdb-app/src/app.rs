use eframe::egui;
use std::sync::mpsc::Receiver;

use crustdb_core::model::{ConnectionId, DriverKind};

use crate::net::ssh::SshSecrets;
use crate::runtime::{Command, Event, ResolvedSecrets, Runtime, TabId};
use crate::state::AppState;
use crate::state::connection_form::ConnectionForm;
use crate::state::dialogs::SessionAuth;
use crate::storage::config::{ConfigStore, ConnectionConfig, FieldError, LoadedConfig};
use crate::storage::secrets::{SecretKind, SecretStore, account};
use crate::ui::{self, Intent};
use secrecy::zeroize::Zeroizing;

pub struct CrustApp {
    state: AppState,
    runtime: Option<Runtime>,
    events: Receiver<Event>,
    store: ConfigStore,
    secrets: Box<dyn SecretStore>,
    form: Option<ConnectionForm>,
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
            form: None,
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
                let secrets = self.stored_secrets(id);
                self.connect(config, secrets);
            }
            Intent::DeleteConnection(id) => self.state.request_delete(id),
            Intent::CancelDialog => {
                let cmd = self.state.dialogs.cancel();
                self.send(cmd);
            }
            Intent::ConfirmSafety => {
                let workspaces = &self.state.workspaces;
                let cmd = self.state.dialogs.submit_safety(
                    |id| {
                        let w = workspaces.iter().find(|w| w.id == id)?;
                        Some(SessionAuth {
                            password: w.session_password.as_ref(),
                            database: w.config.database.as_deref().unwrap_or(""),
                        })
                    },
                    std::time::Instant::now(),
                );
                self.send(cmd);
            }
            Intent::CloseResolve(op) => {
                let cmd = self.state.dialogs.close_with_tx(op);
                self.send(cmd);
            }
            Intent::DiscardFile => {
                let cmd = self.state.dialogs.discard_dirty();
                self.send(cmd);
            }
            // Saving belongs to the editor; the tab stays open, so nothing is lost.
            Intent::SaveFile => {
                self.state.dialogs.pop();
            }
            Intent::AcceptHostKey => {
                let cmd = self.state.dialogs.accept_host_key();
                self.send(cmd);
            }
            Intent::ConfirmDelete => self.confirm_delete(),
            Intent::NewConnection => self.open_form(None, false),
            Intent::ImportUrl => self.open_form(None, true),
            Intent::EditConnection(id) => {
                let c = self.state.config.connections.iter().find(|c| c.id == id);
                self.open_form(c.cloned(), false);
            }
            Intent::FormCancel => self.close_form(),
            Intent::FormImportUrl => {
                if let Some(f) = &mut self.form {
                    let url = Zeroizing::new(std::mem::take(&mut *f.url_input));
                    f.url_error = f.import_url(&url).err().map(|e| e.to_string());
                }
            }
            Intent::FormTest => {
                let ws = self.state.reserve_ws();
                let others = &self.state.config.connections;
                let cmd = self
                    .form
                    .as_mut()
                    .and_then(|f| f.start_test(ws, others).ok());
                if let Some(Command::Connect {
                    ws,
                    config,
                    mut secrets,
                }) = cmd
                {
                    self.fill_missing(config.id, &mut secrets);
                    if let Some(rt) = &self.runtime {
                        rt.send(Command::Connect {
                            ws,
                            config,
                            secrets,
                        });
                    }
                }
            }
            Intent::FormSave => {
                self.save_form();
            }
            Intent::FormConnect => {
                if let Some((config, mut secrets)) = self.save_form() {
                    self.fill_missing(config.id, &mut secrets);
                    self.connect(config, secrets);
                }
            }
            Intent::SwitchWorkspace(id) => self.state.active = Some(id),
            Intent::ToggleNode(n) => {
                if let Some(cmd) = self.tree_mut().and_then(|t| t.toggle(n)) {
                    self.send(cmd);
                }
            }
            Intent::SelectNode(n) => {
                if let Some(t) = self.tree_mut() {
                    t.selected = Some(n);
                }
            }
            Intent::RefreshNode(n) => {
                if let Some(cmd) = self.tree_mut().and_then(|t| t.refresh(n)) {
                    self.send(cmd);
                }
            }
            Intent::RefreshRoot => {
                if let Some(cmd) = self.tree_mut().and_then(|t| t.refresh_root()) {
                    self.send(cmd);
                }
            }
            Intent::Filter(q) => {
                if let Some(t) = self.tree_mut() {
                    t.filter(&q);
                }
            }
            Intent::OpenObject { object } => self.open_object(object),
            Intent::SortHeader { tab, column, shift } => {
                self.table_op(tab, |t| {
                    t.click_header(&column, shift);
                    true
                });
            }
            Intent::NextPage(tab) => self.table_op(tab, |t| t.next_page()),
            Intent::PrevPage(tab) => self.table_op(tab, |t| t.prev_page()),
            Intent::Retry(tab) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab)) {
                    rt.send(t.retry(rt.next_request_id()));
                }
            }
            Intent::ApplyFilters(tab) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab))
                    && let Some(c) = t.apply_filters(rt.next_request_id())
                {
                    rt.send(c);
                }
            }
            Intent::ClearFilters(tab) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab)) {
                    rt.send(t.clear_filters(rt.next_request_id()));
                }
            }
            Intent::SetView(tab, view) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab))
                    && let Some(c) = t.set_view(view, rt.next_request_id())
                {
                    rt.send(c);
                }
            }
            Intent::StopTab(tab) => {
                if let Some(rt) = &self.runtime {
                    rt.send(Command::CancelTab { tab });
                }
            }
        }
    }

    fn open_form(&mut self, config: Option<ConnectionConfig>, url: bool) {
        // One keyring probe per open, never per frame.
        let available = self.secrets.available();
        let mut f = match config {
            Some(c) => ConnectionForm::from_config(&c, available),
            None => ConnectionForm::new(DriverKind::Postgres, available),
        };
        f.show_url = url;
        self.close_form();
        self.form = Some(f);
    }

    /// Drops the form, closing a still-pending test session so it does not leak.
    fn close_form(&mut self) {
        let ws = self.form.take().and_then(|mut f| f.take_pending_test());
        if let (Some(ws), Some(rt)) = (ws, &self.runtime) {
            rt.send(Command::Disconnect { ws });
        }
    }

    fn stored_secrets(&self, id: ConnectionId) -> ResolvedSecrets {
        ResolvedSecrets {
            db: self.secret(id, SecretKind::Db),
            ssh: SshSecrets {
                password: self.secret(id, SecretKind::SshPassword),
                passphrase: self.secret(id, SecretKind::SshPassphrase),
            },
        }
    }

    /// Fields left blank in the form fall back to the keyring (edit of a saved connection).
    fn fill_missing(&self, id: ConnectionId, s: &mut ResolvedSecrets) {
        if s.db.is_none() {
            s.db = self.secret(id, SecretKind::Db);
        }
        if s.ssh.password.is_none() {
            s.ssh.password = self.secret(id, SecretKind::SshPassword);
        }
        if s.ssh.passphrase.is_none() {
            s.ssh.passphrase = self.secret(id, SecretKind::SshPassphrase);
        }
    }

    fn connect(&mut self, config: ConnectionConfig, secrets: ResolvedSecrets) {
        let ws = self.state.open_workspace(config.clone());
        if let Some(w) = self.state.workspace_mut(ws) {
            w.session_password = secrets.db.clone();
        }
        if let Some(rt) = &self.runtime {
            rt.send(Command::Connect {
                ws,
                config,
                secrets,
            });
        }
    }

    fn tree_mut(&mut self) -> Option<&mut crate::state::tree::ObjectTree> {
        let id = self.state.active?;
        self.state.workspace_mut(id).map(|w| &mut w.tree)
    }

    fn open_object(&mut self, object: crustdb_core::model::ObjectRef) {
        let (Some(ws), Some(rt)) = (self.state.active, &self.runtime) else {
            return;
        };
        for c in self.state.open_table(ws, object, rt.next_request_id()) {
            rt.send(c);
        }
    }

    /// Validates, persists the config and keyring secrets; the form stays open on any failure.
    fn save_form(&mut self) -> Option<(ConnectionConfig, ResolvedSecrets)> {
        let form = self.form.as_mut()?;
        let (config, secrets) = match form.to_save(&self.state.config.connections) {
            Ok(r) => r,
            Err(e) => {
                form.errors = e;
                return None;
            }
        };
        form.errors.clear();
        let typed = form.typed_secrets();
        for (kind, v) in &secrets {
            if let Err(e) = self.secrets.set(&account(config.id, *kind), v) {
                tracing::warn!("keyring write failed: {e}");
                if let Some(f) = &mut self.form {
                    f.errors = vec![FieldError {
                        field: "password",
                        message: format!("could not save to keyring: {e}"),
                    }];
                }
                return None;
            }
        }
        let mut all = self.state.config.connections.clone();
        match all.iter_mut().find(|c| c.id == config.id) {
            Some(slot) => *slot = config.clone(),
            None => all.push(config.clone()),
        }
        if let Err(e) = self.store.save(&all) {
            if let Some(f) = &mut self.form {
                f.errors = vec![FieldError {
                    field: "name",
                    message: format!("could not save connections: {e}"),
                }];
            }
            return None;
        }
        self.state.config.connections = all;
        if !config.save_password {
            for kind in [
                SecretKind::Db,
                SecretKind::SshPassword,
                SecretKind::SshPassphrase,
            ] {
                if let Err(e) = self.secrets.delete(&account(config.id, kind)) {
                    tracing::warn!("keyring delete failed: {e}");
                }
            }
        }
        self.close_form();
        Some((config, typed))
    }

    /// Applies `f` to the table tab and refetches its page if `f` reports a change.
    fn table_op(&mut self, tab: TabId, f: impl FnOnce(&mut crate::state::TableTab) -> bool) {
        let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab)) else {
            return;
        };
        if f(t) {
            rt.send(t.fetch(rt.next_request_id()));
        }
    }

    fn send(&self, cmd: impl Into<Option<Command>>) {
        if let (Some(rt), Some(cmd)) = (&self.runtime, cmd.into()) {
            rt.send(cmd);
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
            if let Some(f) = &mut self.form
                && f.on_event(&e)
                && let Event::Connected { ws, .. } | Event::ConnectFailed { ws, .. } = &e
                && let Some(rt) = &self.runtime
            {
                rt.send(Command::Disconnect { ws: *ws });
            }
            self.state.apply(e);
        }
        let cmds: Vec<_> = self
            .state
            .workspaces
            .iter_mut()
            .filter(|w| matches!(w.status, crate::state::Status::Online(_)))
            .filter_map(|w| w.tree.load_roots())
            .collect();
        for c in cmds {
            self.send(c);
        }
        if let Some(rt) = &self.runtime {
            for c in self.state.take_fetches(|| rt.next_request_id()) {
                rt.send(c);
            }
        }
        for cmd in self.state.take_commands() {
            self.send(cmd);
        }
        let mut intents = vec![];
        if self.state.active_online().is_some() {
            egui::Panel::left("sidebar").show(ui, |ui| {
                intents.extend(ui::sidebar::show(ui, &self.state));
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(f) = &mut self.form {
                intents.extend(ui::connection_form::show(ui, f));
                return;
            }
            let tab = self
                .state
                .active
                .and_then(|ws| self.state.workspace_mut(ws))
                .and_then(|w| w.active_table_mut());
            match tab {
                Some(t) => intents.extend(ui::grid::show_tab(ui, t)),
                None => intents.extend(ui::home::show(ui, &self.state)),
            }
        });
        intents.extend(ui::confirm_dialog::show(ui.ctx(), &mut self.state.dialogs));
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
