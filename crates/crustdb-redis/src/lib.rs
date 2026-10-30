//! Redis driver for crustdb.

pub mod classify;
#[allow(dead_code)] // wired by the driver in the next commit
mod connect;
mod decode;
#[allow(dead_code)] // wired by the driver in the next commit
mod session;
mod tls;
pub mod tokenize;
