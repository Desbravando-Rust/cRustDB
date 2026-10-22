#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)]
//! Drives the whole app against `FakeDriver`: connect, open a table, see the grid.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crustdb_app::app::CrustApp;
use crustdb_app::runtime::DriverRegistry;
use crustdb_app::storage::config::{ConfigStore, ConnectionConfig, LoadedConfig};
use crustdb_app::storage::secrets::MemoryStore;
use crustdb_app::ui::Intent;
use crustdb_core::driver::{TlsConfig, TlsMode};
use crustdb_core::model::{
    Cell, ColumnMeta, ConnectionId, DriverKind, ObjectKind, ObjectRef, TypeCategory,
};
use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn text(s: &str) -> Cell {
    Cell::Text {
        value: s.into(),
        truncated: false,
    }
}

fn fake() -> Arc<FakeDriver> {
    FakeDriver::new(FakeScript {
        columns: vec![ColumnMeta {
            name: "email".into(),
            type_name: "text".into(),
            category: TypeCategory::Text,
            nullable: Some(true),
            is_primary_key: false,
        }],
        rows: vec![Box::new([text("alice@example.com")])],
        ..FakeScript::default()
    })
}

type Started = (
    Harness<'static>,
    Rc<RefCell<Option<CrustApp>>>,
    tempfile::TempDir,
);

fn start() -> Started {
    let dir = tempfile::tempdir().unwrap();
    let driver = fake();
    let config = LoadedConfig {
        connections: vec![ConnectionConfig {
            id: ConnectionId(uuid::Uuid::from_u128(1)),
            name: "smoke".into(),
            driver: DriverKind::Postgres,
            host: "localhost".into(),
            port: 5432,
            user: "u".into(),
            database: None,
            save_password: false,
            tls: TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
            },
            ssh: None,
            page_size: 300,
            editor_row_limit: 1000,
            safety_threshold: 1000,
        }],
        read_only: false,
        warning: None,
    };
    let mut config = Some(config);
    let path = dir.path().join("config.json");
    let hist = dir.path().join("history");

    let app: Rc<RefCell<Option<CrustApp>>> = Rc::default();
    let slot = app.clone();
    let h = Harness::builder()
        .with_accessibility_check(false)
        // the sidebar arrows (U+25B8) are not in the installed fonts
        .allow_missing_glyphs()
        .build_ui(move |ui| {
            let mut a = slot.borrow_mut();
            if a.is_none() {
                // Fonts apply on the pass after `set_fonts`; draw from the next one.
                *a = Some(CrustApp::with_ctx(
                    ui.ctx(),
                    config.take().unwrap(),
                    ConfigStore { path: path.clone() },
                    Box::<MemoryStore>::default(),
                    DriverRegistry::default().with(driver.clone()),
                    hist.clone(),
                ));
                return;
            }
            if let Some(app) = a.as_mut() {
                app.show(ui);
            }
        });
    (h, app, dir)
}

fn wait_for(h: &mut Harness<'_>, label: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_label(label).is_none() {
        assert!(Instant::now() < end, "timed out waiting for {label:?}");
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
}

#[test]
fn connect_and_open_table_renders_grid() {
    let (mut h, app, _dir) = start();
    h.run();
    h.get_by_label("Connect").click();
    wait_for(&mut h, "New query"); // sidebar is up once the workspace is online

    let object = ObjectRef {
        database: "fake".into(),
        schema: Some("public".into()),
        name: "users".into(),
        kind: ObjectKind::Table,
    };
    app.borrow_mut()
        .as_mut()
        .unwrap()
        .handle(Intent::OpenObject { object });
    wait_for(&mut h, "alice@example.com");
    h.get_by_label("users"); // tab strip entry
}

#[test]
fn run_query_shows_result_and_messages() {
    let (mut h, _app, _dir) = start();
    h.run();
    h.get_by_label("Connect").click();
    wait_for(&mut h, "New query");
    h.get_by_label("New query").click();
    h.run();
    let editor = h.get_by_role(eframe::egui::accesskit::Role::MultilineTextInput);
    editor.click();
    h.run();
    h.get_by_role(eframe::egui::accesskit::Role::MultilineTextInput)
        .type_text("select email from users");
    h.run();
    h.get_by_label("Run (Ctrl+Enter)").click();
    wait_for(&mut h, "alice@example.com");
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_label_contains("#1").is_none() {
        if Instant::now() > end {
            h.get_by_label_contains("#1");
        }
        h.get_by_label("Messages").click();
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
}
