use eframe::egui;
use std::sync::mpsc::Receiver;

use crustdb_core::model::{ConnectionId, DriverKind};

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
mod files;
mod form;
mod frame;

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
    ctx: egui::Context,
    picker: ui::picker::Picker,
    history: Option<ui::history_panel::HistoryPanel>,
    history_store: crate::storage::history::HistoryStore,
}

impl CrustApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: LoadedConfig,
        store: ConfigStore,
        secrets: Box<dyn SecretStore>,
        drivers: crate::runtime::DriverRegistry,
    ) -> Self {
        Self::with_ctx(
            &cc.egui_ctx,
            config,
            store,
            secrets,
            drivers,
            crate::storage::paths::data_dir().join("history"),
        )
    }

    /// Construction without a window, so UI tests can drive the app.
    pub fn with_ctx(
        egui_ctx: &egui::Context,
        mut config: LoadedConfig,
        store: ConfigStore,
        secrets: Box<dyn SecretStore>,
        drivers: crate::runtime::DriverRegistry,
        history_dir: std::path::PathBuf,
    ) -> Self {
        ui::theme::install_theme(egui_ctx);
        egui_extras::install_image_loaders(egui_ctx);
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
            ctx: egui_ctx.clone(),
            picker: ui::picker::Picker::default(),
            history: None,
            history_store: crate::storage::history::HistoryStore { dir: history_dir },
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
                if let Some(ws) = self.state.live_workspace(id) {
                    self.state.active = Some(ws);
                    return;
                }
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
            Intent::Label(op) => self.apply_label_op(op),
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
                // Discarding the file still goes through the transaction guard.
                let cmd = self.state.discard_dirty_and_close();
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
            Intent::AcceptHostKey => {
                let cmd = self.state.dialogs.accept_host_key();
                self.send(cmd);
            }
            Intent::ConfirmDelete => self.confirm_delete(),
            Intent::NewConnection => self.open_form(None),
            Intent::EditConnection(id) => {
                let c = self.state.config.connections.iter().find(|c| c.id == id);
                if let Some(c) = c.cloned() {
                    self.open_form(Some(c));
                }
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
            Intent::Disconnect(ws) => {
                self.state.close_workspace(ws);
                self.send(Command::Disconnect { ws });
            }
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
                self.reload_home();
            }
            Intent::RefreshRoot => {
                if let Some(cmd) = self.tree_mut().and_then(|t| t.refresh_root()) {
                    self.send(cmd);
                }
                self.reload_home();
            }
            Intent::Filter(q) => {
                if let Some(t) = self.tree_mut() {
                    t.filter(&q);
                }
            }
            Intent::OpenObject { object } => self.open_object(object),
            Intent::SortHeader { tab, column, shift } => {
                self.table_op(tab, |t| t.click_header(&column, shift));
            }
            Intent::NextPage(tab) => self.table_op(tab, |t| t.next_page()),
            Intent::PrevPage(tab) => self.table_op(tab, |t| t.prev_page()),
            Intent::Retry(tab) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab))
                    && let Some(c) = t.retry(rt.next_request_id())
                {
                    rt.send(c);
                }
            }
            Intent::CommitEdit(tab) => {
                if let Some(t) = self.state.table_mut(tab) {
                    t.commit_edit();
                }
            }
            Intent::SetNull { tab, row, col } => {
                if let Some(t) = self.state.table_mut(tab) {
                    t.set_null(row, col);
                }
            }
            Intent::FollowFk { tab, row, col } => {
                if let Some(rt) = &self.runtime {
                    for c in self.state.follow_fk(tab, row, col, rt.next_request_id()) {
                        self.send(c);
                    }
                }
            }
            Intent::SavePending(tab) => {
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab))
                    && let Some(c) = t.save_pending(rt.next_request_id())
                {
                    rt.send(c);
                }
            }
            Intent::DiscardPending(tab) => {
                if let Some(t) = self.state.table_mut(tab) {
                    t.discard_pending();
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
                if let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab))
                    && let Some(c) = t.clear_filters(rt.next_request_id())
                {
                    rt.send(c);
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
            other => self.handle_files(other),
        }
    }

    fn open_form(&mut self, config: Option<ConnectionConfig>) {
        // One keyring probe per open, never per frame.
        let available = self.secrets.available();
        let f = match config {
            Some(c) => ConnectionForm::from_config(&c, available),
            None => ConnectionForm::new(DriverKind::Postgres, available),
        };
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

    /// A refresh may drop the home's tables folder; request it again.
    fn reload_home(&mut self) {
        let Some(w) = self
            .state
            .active
            .and_then(|id| self.state.workspace_mut(id))
        else {
            return;
        };
        for c in w.expand_home() {
            self.send(c);
        }
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
}

impl eframe::App for CrustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
        crate::metrics::trace_first_frame();
    }

    fn on_exit(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown();
        }
    }
}
