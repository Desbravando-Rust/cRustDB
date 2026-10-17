use crustdb_core::driver::{ConnectTarget, SessionPurpose, TlsMode};
use crustdb_core::error::{DriverError, ErrorKind};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, IoError, Opts, OptsBuilder, SslOpts, TlsError};
use secrecy::ExposeSecret;

use crate::session::MysqlSession;

pub(crate) fn err(kind: ErrorKind, message: impl Into<String>) -> DriverError {
    DriverError {
        kind,
        message: message.into(),
    }
}

/// `None` for `Disable`. No LOCAL INFILE handler is ever registered.
fn ssl_opts(t: &ConnectTarget) -> Option<SslOpts> {
    let mut ssl = SslOpts::default();
    match t.tls.mode {
        TlsMode::Disable => return None,
        TlsMode::Require => ssl = ssl.with_danger_accept_invalid_certs(true),
        // ponytail: mysql_async 0.37.1 matches rustls' Display text "NotValidForName", which rustls
        // 0.23.46 changed, so this flag never fires: VerifyCa behaves as VerifyFull on MySQL (fail-closed).
        TlsMode::VerifyCa => ssl = ssl.with_danger_skip_domain_validation(true),
        TlsMode::VerifyFull => {}
    }
    // A configured CA replaces the built-in roots, as for PostgreSQL.
    if let Some(p) = &t.tls.ca_path {
        ssl = ssl
            .with_root_certs(vec![p.clone().into()])
            .with_disable_built_in_roots(true);
    }
    if t.connect_addr.is_some() {
        ssl = ssl.with_danger_tls_hostname_override(Some(t.host.clone()));
    }
    Some(ssl)
}

fn opts(t: &ConnectTarget) -> Opts {
    let (host, port) = match t.connect_addr {
        Some(a) => (a.ip().to_string(), a.port()),
        None => (t.host.clone(), t.port),
    };
    OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .user(Some(&t.user))
        .pass(t.password.as_ref().map(|p| p.expose_secret()))
        .db_name(t.database.as_deref())
        .prefer_socket(false)
        .ssl_opts(ssl_opts(t))
        .into()
}

pub(crate) fn map_err(e: mysql_async::Error) -> DriverError {
    use mysql_async::Error::{Io, Server};
    let message = e.to_string();
    match &e {
        Server(s) => {
            let kind = match s.code {
                1044 | 1045 | 1698 => ErrorKind::Auth,
                1317 => ErrorKind::Cancelled,
                c => ErrorKind::Query {
                    code: Some(c.to_string()),
                    position: None,
                },
            };
            err(kind, s.message.clone())
        }
        Io(IoError::Tls(TlsError::Tls(t))) => tls_error(t, message),
        Io(IoError::Tls(_)) => err(ErrorKind::Tls, message),
        Io(IoError::Io(io)) => {
            let tls = io.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>());
            if let Some(t) = tls {
                tls_error(t, message)
            } else if io.kind() == std::io::ErrorKind::TimedOut {
                err(ErrorKind::Timeout, "connection timed out")
            } else {
                err(ErrorKind::Connect, message)
            }
        }
        _ => err(ErrorKind::Connect, message),
    }
}

/// D39: a server without TLS 1.2+ gets a fixed, actionable message.
fn tls_error(e: &rustls::Error, message: String) -> DriverError {
    use rustls::{
        AlertDescription::ProtocolVersion, Error::*,
        PeerIncompatible::ServerDoesNotSupportTls12Or13,
    };
    let no_tls12 = matches!(
        e,
        PeerIncompatible(ServerDoesNotSupportTls12Or13) | AlertReceived(ProtocolVersion)
    );
    let message = if no_tls12 {
        "server does not support TLS 1.2+; use an SSH tunnel or disable TLS".into()
    } else {
        message
    };
    err(ErrorKind::Tls, message)
}

pub(crate) async fn connect(
    t: &ConnectTarget,
    purpose: SessionPurpose,
) -> Result<MysqlSession, DriverError> {
    // Both rustls providers are compiled in workspace-wide, so there is no implicit default.
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
    let opts = opts(t);
    let work = async {
        let mut conn = Conn::new(opts.clone()).await.map_err(map_err)?;
        if purpose != SessionPurpose::Editor {
            conn.query_drop("SET SESSION TRANSACTION READ ONLY")
                .await
                .map_err(map_err)?;
        }
        let raw: String = conn
            .query_first("SELECT VERSION()")
            .await
            .map_err(map_err)?
            .unwrap_or_default();
        Ok(MysqlSession::new(conn, opts, raw))
    };
    tokio::time::timeout(t.connect_timeout, work)
        .await
        .map_err(|_| err(ErrorKind::Timeout, "connection timed out"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_switches_to_a_server_announced_unix_socket() {
        let t = ConnectTarget {
            host: "h".into(),
            port: 3306,
            connect_addr: None,
            user: "u".into(),
            password: None,
            database: None,
            tls: crustdb_core::driver::TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
            },
            connect_timeout: std::time::Duration::from_secs(1),
        };
        assert!(!opts(&t).prefer_socket());
    }

    #[test]
    fn old_tls_maps_to_d39_message() {
        use rustls::{AlertDescription, PeerIncompatible};
        const D39: &str = "server does not support TLS 1.2+; use an SSH tunnel or disable TLS";
        for e in [
            rustls::Error::PeerIncompatible(PeerIncompatible::ServerDoesNotSupportTls12Or13),
            rustls::Error::AlertReceived(AlertDescription::ProtocolVersion),
        ] {
            let d = tls_error(&e, "x".into());
            assert_eq!((d.kind, d.message.as_str()), (ErrorKind::Tls, D39));
        }
        let e = rustls::Error::PeerIncompatible(PeerIncompatible::NoCipherSuitesInCommon);
        assert_eq!(tls_error(&e, "x".into()).message, "x");
    }
}
