use eframe::egui;
use std::sync::mpsc::Receiver;

use crustdb_core::model::{ConnectionId, DriverKind};

use crate::keymap::{Action, keymap};
use crate::metrics::ProcSampler;
use crate::net::ssh::SshSecrets;
use crate::runtime::{Command, Event, ResolvedSecrets, Runtime};
use crate::state::AppState;
use crate::state::connection_form::ConnectionForm;
use crate::state::dialogs::SessionAuth;
use crate::state::status::StatusBoard;
use crate::storage::config::{ConfigStore, ConnectionConfig, LoadedConfig};
use crate::storage::secrets::{SecretKind, SecretStore, account};
use crate::ui::{self, Intent};
use secrecy::zeroize::Zeroizing;

mod actions;
mod form;

pub struct CrustApp {
    state: AppState,
    runtime: Option<Runtime>,
    events: Receiver<Event>,
    store: ConfigStore,
    secrets: Box<dyn SecretStore>,
    form: Option<ConnectionForm>,
    status: StatusBoard,
    sampler: ProcSampler,
    /// Last press landed in the sidebar: F5 and Space then belong to it, not to the grid.
    sidebar_rect: Option<egui::Rect>,
    sidebar_focus: bool,
}

impl CrustApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: LoadedConfig,
        store: ConfigStore,
        secrets: Box<dyn SecretStore>,
        drivers: crate::runtime::DriverRegistry,
    ) -> Self {
        Self::with_ctx(&cc.egui_ctx, config, store, secrets, drivers)
    }

    /// Construction without a window, so UI tests can drive the app.
    pub fn with_ctx(
        egui_ctx: &egui::Context,
        mut config: LoadedConfig,
        store: ConfigStore,
        secrets: Box<dyn SecretStore>,
        drivers: crate::runtime::DriverRegistry,
    ) -> Self {
        ui::theme::install_theme(egui_ctx);
        if !secrets.available() {
            config.warning.get_or_insert_with(|| {
                "OS keyring unavailable: passwords cannot be saved".to_owned()
            });
        }
        let ctx = egui_ctx.clone();
        let (runtime, events) =
            Runtime::start(std::sync::Arc::new(move || ctx.request_repaint()), drivers);
        Self {
            state: AppState::new(config),
            runtime: Some(runtime),
            events,
            store,
            secrets,
            form: None,
            status: StatusBoard::default(),
            sampler: ProcSampler::default(),
            sidebar_rect: None,
            sidebar_focus: false,
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

    pub fn handle(&mut self, intent: Intent) {
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
                self.send_close(cmd);
            }
            Intent::DiscardFile => {
                let cmd = self.state.dialogs.discard_dirty();
                self.send_close(cmd);
            }
            Intent::ActivateTab(tab) => {
                if let Some(w) = self.active_ws() {
                    w.active_tab = Some(tab);
                }
            }
            Intent::CloseTab(tab) => {
                if let Some(ws) = self.state.active {
                    let cmd = self.state.close_tab(ws, tab);
                    self.send_close(cmd);
                }
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
            Intent::NewQuery => {
                if let Some(ws) = self.state.active {
                    let cmd = self.state.open_query(ws, None);
                    self.send(cmd);
                }
            }
            Intent::RunQuery { tab, run, all } => {
                if let Some(req) = self.runtime.as_ref().map(|rt| rt.next_request_id()) {
                    let cmd = self.state.run_query(tab, run, all, req);
                    self.send(cmd);
                }
            }
            Intent::StopTab(tab) => {
                let cmd = match self.state.query_mut(tab) {
                    Some(q) => q.stop(),
                    None => Some(Command::CancelTab { tab }),
                };
                self.send(cmd);
            }
            Intent::Tx { tab, op } => {
                let cmd = self.state.query_mut(tab).map(|q| q.tx(op));
                self.send(cmd);
            }
            Intent::UseDatabase { tab, database } => {
                let cmd = self
                    .state
                    .query_mut(tab)
                    .and_then(|q| q.use_database(database));
                self.send(cmd);
            }
            Intent::LoadMore(tab) => {
                let cmd = self.state.query_mut(tab).and_then(|q| q.load_more());
                self.send(cmd);
            }
            Intent::ExportResult(tab) => {
                let cmd = self.state.query_mut(tab).and_then(|q| q.export());
                self.send(cmd);
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

    fn send(&mut self, cmd: impl Into<Option<Command>>) {
        if let (Some(rt), Some(cmd)) = (&self.runtime, cmd.into()) {
            let now = std::time::Instant::now();
            self.status.on_command(&cmd, &self.state, now);
            self.state.note_sent(&cmd);
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
        self.show(ui);
    }

    fn on_exit(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown();
        }
    }
}

impl CrustApp {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let now = std::time::Instant::now();
        while let Ok(e) = self.events.try_recv() {
            self.status.on_event(&e, &self.state, now);
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
        self.status.tick(now, || self.sampler.sample());
        ui.ctx()
            .request_repaint_after(crate::state::status::SAMPLE_EVERY);
        let mut intents = vec![];
        let focus = self.focus(ui.ctx());
        if let Some(a) = ui.input(|i| keymap(i, focus)) {
            if a == Action::RowDetail {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
            }
            intents.extend(self.on_action(a, focus));
        }
        if let Some(m) = self.status.current(&self.state) {
            egui::Panel::bottom("status_bar").show(ui, |ui| ui::status_bar::show(ui, m));
        }
        self.sidebar_rect = None;
        if self.state.active_online().is_some() {
            let r = egui::Panel::left("sidebar").show(ui, |ui| {
                intents.extend(ui::sidebar::show(ui, &self.state));
            });
            self.sidebar_rect = Some(r.response.rect);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(f) = &mut self.form {
                intents.extend(ui::connection_form::show(ui, f));
                return;
            }
            if let Some(w) = self.state.active_online() {
                intents.extend(ui::tab_bar::show(ui, w));
            }
            let ws = self
                .state
                .active
                .and_then(|ws| self.state.workspace_mut(ws));
            if let Some(q) = ws.and_then(|w| w.active_query_mut()) {
                egui::Panel::bottom(("results", q.id.0))
                    .resizable(true)
                    .default_size(240.0)
                    .show(ui, |ui| {
                        intents.extend(ui::results::show(ui, q));
                    });
                intents.extend(ui::editor::show(ui, q));
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
        self.status.frame_done(std::time::Instant::now());
    }
}
