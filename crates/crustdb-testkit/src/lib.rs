#![allow(clippy::unwrap_used, clippy::expect_used)] // test-only crate: failing loudly is the intent
mod certs;
mod containers;
pub mod fake_driver;
pub mod fake_store;
pub use certs::{TestCerts, certs_for};
pub use containers::{
    MysqlContainer, PgContainer, SshContainer, start_mysql, start_postgres,
    start_postgres_tls_only, start_sshd,
};
