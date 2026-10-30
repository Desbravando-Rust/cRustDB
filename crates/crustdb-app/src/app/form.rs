//! Connection form persistence for `CrustApp`.
use super::CrustApp;
use crate::runtime::ResolvedSecrets;
use crate::storage::config::{
    ConnectionConfig, FieldError, Label, delete_label, rename_label, validate_label,
};
use crate::storage::secrets::{SecretKind, account};
use crate::ui::labels::LabelOp;

impl CrustApp {
    /// Validates, persists the config and keyring secrets; the form stays open on any failure.
    pub(super) fn save_form(&mut self) -> Option<(ConnectionConfig, ResolvedSecrets)> {
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
        if let Err(e) = self.store.save(&all, &self.state.config.labels) {
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

    /// Applies a catalog edit (propagating to connections) and saves; keeps the old state on failure.
    pub(super) fn apply_label_op(&mut self, op: LabelOp) {
        let cfg = &mut self.state.config;
        let (mut conns, mut cat) = (cfg.connections.clone(), cfg.labels.clone());
        match op {
            LabelOp::Add(name) => match validate_label(name.trim(), &cat) {
                Ok(()) => cat.push(Label {
                    name: name.trim().to_string(),
                    color: [128, 128, 128],
                }),
                Err(e) => return cfg.warning = Some(format!("label: {e}")),
            },
            LabelOp::Rename { old, new } => {
                let new = new.trim().to_string();
                let others: Vec<Label> = cat.iter().filter(|l| l.name != old).cloned().collect();
                match validate_label(&new, &others) {
                    Ok(()) => {
                        rename_label(&mut conns, &mut cat, &old, &new);
                        crate::ui::labels::rename_filter(&self.ctx, &old, &new);
                    }
                    Err(e) => return cfg.warning = Some(format!("label: {e}")),
                }
            }
            LabelOp::Color { name, color } => {
                // In memory only; `Persist` saves once the drag ends.
                return cfg
                    .labels
                    .iter_mut()
                    .filter(|l| l.name == name)
                    .for_each(|l| l.color = color);
            }
            LabelOp::Persist => {}
            LabelOp::Delete(name) => delete_label(&mut conns, &mut cat, &name),
        }
        match self.store.save(&conns, &cat) {
            Ok(()) => {
                cfg.connections = conns;
                cfg.labels = cat;
            }
            Err(e) => cfg.warning = Some(format!("could not save labels: {e}")),
        }
    }
}
