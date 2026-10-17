#![allow(clippy::unwrap_used, clippy::expect_used)] // test-only crate: failing loudly is the intent
mod certs;
mod containers;
pub use certs::{TestCerts, certs_for};
pub use containers::{
    MysqlContainer, PgContainer, SshContainer, start_mysql, start_postgres, start_sshd,
};
