//! Client certificate (mTLS) rows of the connection form.
use crustdb_core::driver::TlsMode;
use crustdb_core::model::DriverKind;
use eframe::egui;

use super::connection_form::{error_for, path_edit};
use crate::state::connection_form::ConnectionForm;

/// Only for PostgreSQL/MySQL with TLS on.
pub fn fields(ui: &mut egui::Ui, f: &mut ConnectionForm) {
    if f.draft.driver == DriverKind::Redis || f.draft.tls.mode == TlsMode::Disable {
        return;
    }
    ui.label("Client certificate").on_hover_text(
        "PEM file with the client certificate chain, for servers that require mTLS. Empty = none.",
    );
    path_edit(ui, &mut f.draft.tls.client_cert_path);
    ui.end_row();
    error_for(ui, f, "client_cert");
    ui.end_row();

    ui.label("Client key")
        .on_hover_text("PEM file with the unencrypted private key of the client certificate.");
    path_edit(ui, &mut f.draft.tls.client_key_path);
    ui.end_row();
    error_for(ui, f, "client_key");
    ui.end_row();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn shows_cert_and_key_rows() {
        // Arrange
        let mut f = ConnectionForm::new(DriverKind::Postgres, false);
        f.draft.tls.mode = TlsMode::Require;
        let mut h = Harness::builder()
            .with_accessibility_check(false)
            .build_ui(|ui| fields(ui, &mut f));

        // Act
        h.run();

        // Assert
        h.get_by_label("Client certificate");
        h.get_by_label("Client key");
    }
}
