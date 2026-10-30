//! Connection form persistence for `CrustApp`.
use super::CrustApp;
use crate::runtime::ResolvedSecrets;
use crate::storage::config::{ConnectionConfig, FieldError};
use crate::storage::secrets::{SecretKind, account};

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
}
