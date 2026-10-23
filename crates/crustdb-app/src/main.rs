use std::fs::OpenOptions;
use std::sync::Arc;

use crustdb_app::app::CrustApp;
use crustdb_app::runtime::DriverRegistry;
use crustdb_app::storage::config::{ConfigStore, LoadedConfig};
use crustdb_app::storage::{paths, secrets};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

const APP_NAME: &str = "cRustDB";
/// Baked in at build time; unset means reporting is disabled.
const SENTRY_DSN: &str = match option_env!("SENTRY_DSN") {
    Some(dsn) => dsn,
    None => "",
};

fn init_logging() {
    let dir = paths::state_dir();
    let file = std::fs::create_dir_all(&dir)
        .and_then(|()| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crustdb.log"))
        })
        .ok();
    let filter =
        || EnvFilter::try_from_env("CRUSTDB_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter());
    let file = file.map(|f| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(Arc::new(f))
            .with_filter(filter())
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .init();
}

fn main() -> anyhow::Result<()> {
    let _ = crustdb_app::metrics::PROCESS_START.set(std::time::Instant::now());
    // ponytail: panics only; `SENTRY_DSN=""` disables reporting.
    let mut sentry_options = sentry::ClientOptions::default();
    sentry_options.release = sentry::release_name!();
    sentry_options.send_default_pii = false;
    let _sentry = sentry::init((
        std::env::var("SENTRY_DSN").unwrap_or_else(|_| SENTRY_DSN.to_owned()),
        sentry_options,
    ));
    init_logging();
    let store = ConfigStore {
        path: paths::config_dir().join("connections.json"),
    };
    let config = store.load().unwrap_or_else(|e| {
        tracing::error!("loading connections failed: {e}");
        LoadedConfig {
            connections: vec![],
            read_only: true,
            warning: Some(format!("could not load connections: {e}")),
        }
    });
    let drivers = DriverRegistry::default()
        .with(Arc::new(crustdb_postgres::PostgresDriver))
        .with(Arc::new(crustdb_mysql::MysqlDriver));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title(APP_NAME),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            Ok(Box::new(CrustApp::new(
                cc,
                config,
                store,
                secrets::detect_secret_store(),
                drivers,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
