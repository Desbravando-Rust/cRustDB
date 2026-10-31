//! Connecting: ACL user, database index, TLS and timeouts.

use crustdb_core::driver::{ConnectTarget, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, FromRedisValue};
use redis::{RedisConnectionInfo, RedisError};
use rustls::pki_types::ServerName;
use secrecy::ExposeSecret;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::decode;
use crate::session::RedisSession;
use crate::tls::{self, err};

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

/// Where to open the socket: the tunnel address when there is one, else `host:port`.
pub(crate) fn dial_addr(t: &ConnectTarget) -> String {
    match t.connect_addr {
        Some(a) => a.to_string(),
        None if t.host.contains(':') => format!("[{}]:{}", t.host, t.port),
        None => format!("{}:{}", t.host, t.port),
    }
}

/// The TLS name is always the real host, never the tunnel address.
fn tls_server_name(t: &ConnectTarget) -> Result<ServerName<'static>, DriverError> {
    ServerName::try_from(t.host.clone())
        .map_err(|e| err(ErrorKind::Tls, format!("invalid TLS server name: {e}")))
}

fn database_index(t: &ConnectTarget) -> Result<u32, DriverError> {
    match t.database.as_deref() {
        None | Some("") => Ok(0),
        Some(d) => d
            .parse()
            .map_err(|_| err(ErrorKind::Connect, "database must be a numeric index")),
    }
}

/// An empty user means the `default` ACL user. Without a password no `AUTH` is sent.
pub(crate) fn connection_info(t: &ConnectTarget) -> Result<RedisConnectionInfo, DriverError> {
    let mut info = RedisConnectionInfo::default().set_db(database_index(t)?.into());
    if let Some(p) = &t.password {
        let user = if t.user.is_empty() {
            "default"
        } else {
            &t.user
        };
        info = info.set_username(user).set_password(p.expose_secret());
    }
    Ok(info)
}

pub(crate) fn map_err(e: RedisError) -> DriverError {
    let code = e.code().map(str::to_owned);
    let message = match (&code, e.detail()) {
        (Some(c), Some(d)) => format!("{c} {d}"),
        (Some(c), None) => c.clone(),
        _ => e.to_string(),
    };
    let kind = if e.kind() == redis::ErrorKind::AuthenticationFailed
        || matches!(code.as_deref(), Some("NOAUTH" | "WRONGPASS"))
    {
        ErrorKind::Auth
    } else if e.is_timeout() {
        ErrorKind::Timeout
    } else if e.is_io_error() {
        ErrorKind::Connect
    } else if code.is_some() {
        ErrorKind::Query {
            code,
            position: None,
        }
    } else {
        ErrorKind::Protocol
    };
    err(kind, message)
}

/// The server answered with an error reply (as opposed to I/O or protocol failures).
pub(crate) fn is_server_error(e: &RedisError) -> bool {
    e.code().is_some()
}

pub(crate) fn from_value<T: FromRedisValue>(v: redis::Value) -> Result<T, DriverError> {
    T::from_redis_value(v).map_err(|e| map_err(e.into()))
}

/// A failed handshake carries the `rustls::Error` inside an `io::Error`.
fn handshake_error(e: std::io::Error) -> DriverError {
    let message = format!("TLS handshake failed: {e}");
    match e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
        Some(t) => tls::tls_error(t, message),
        None => err(ErrorKind::Tls, message),
    }
}

async fn open(t: &ConnectTarget) -> Result<Box<dyn AsyncStream>, DriverError> {
    let tls = tls::client_config(&t.tls)?;
    let tcp = TcpStream::connect(dial_addr(t))
        .await
        .map_err(|e| err(ErrorKind::Connect, e.to_string()))?;
    // Best effort: only affects latency.
    let _ = tcp.set_nodelay(true);
    Ok(match tls {
        Some(cfg) => Box::new(
            TlsConnector::from(cfg)
                .connect(tls_server_name(t)?, tcp)
                .await
                .map_err(handshake_error)?,
        ),
        None => Box::new(tcp),
    })
}

/// One timeout covers TCP, TLS, `AUTH`/`SELECT` and the version probe.
pub(crate) async fn connect(
    t: &ConnectTarget,
    purpose: SessionPurpose,
) -> Result<RedisSession, DriverError> {
    let info = connection_info(t)?;
    let work = async {
        let stream = open(t).await?;
        // The default 500 ms response timeout would break slow commands; cancelling means dropping.
        let config = AsyncConnectionConfig::new().set_response_timeout(None);
        let (conn, driver) = MultiplexedConnection::new_with_config(&info, stream, config)
            .await
            .map_err(map_err)?;
        let mut session =
            RedisSession::new(conn, tokio::spawn(driver), purpose, database_index(t)?);
        let raw = session.call(["INFO", "server"]).await?;
        let raw: String = from_value(raw)?;
        session.set_version(decode::parse_info_version(&raw));
        Ok(session)
    };
    tokio::time::timeout(t.connect_timeout, work)
        .await
        .map_err(|_| err(ErrorKind::Timeout, "connection timed out"))?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use crustdb_core::driver::{TlsConfig, TlsMode};
    use secrecy::SecretString;

    use super::*;

    fn target() -> ConnectTarget {
        ConnectTarget {
            host: "localhost".into(),
            port: 6379,
            connect_addr: None,
            user: String::new(),
            password: None,
            database: None,
            tls: TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
                client_cert_path: None,
                client_key_path: None,
            },
            connect_timeout: Duration::from_secs(1),
        }
    }

    fn server_error(line: &str) -> RedisError {
        match redis::parse_redis_value(line.as_bytes()).unwrap() {
            redis::Value::ServerError(e) => e.into(),
            other => panic!("not a server error: {other:?}"),
        }
    }

    #[test]
    fn dials_connect_addr_but_keeps_host_for_tls() {
        // Arrange
        let mut t = target();
        t.connect_addr = Some("127.0.0.1:7000".parse().unwrap());

        // Act & Assert
        assert_eq!(dial_addr(&t), "127.0.0.1:7000");
        assert_eq!(
            tls_server_name(&t).unwrap(),
            ServerName::try_from("localhost").unwrap()
        );
        t.connect_addr = None;
        assert_eq!(dial_addr(&t), "localhost:6379");
    }

    #[test]
    fn database_must_be_u32_index() {
        // Arrange
        let mut t = target();

        // Act & Assert
        for bad in ["-1", "abc", "4294967296"] {
            t.database = Some(bad.into());
            let e = connection_info(&t).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Connect, "{bad}");
            assert!(e.message.contains("database must be a numeric index"));
        }
        t.database = Some("15".into());
        assert_eq!(connection_info(&t).unwrap().db(), 15);
    }

    #[test]
    fn acl_user_and_db_go_to_connection_info() {
        // Arrange
        let mut t = target();
        t.user = "reader".into();
        t.password = Some(SecretString::from("pw"));
        t.database = Some("3".into());

        // Act
        let info = connection_info(&t).unwrap();
        t.user = String::new();
        let anon = connection_info(&t).unwrap();

        // Assert
        assert_eq!(info.username(), Some("reader"));
        assert_eq!(info.password(), Some("pw"));
        assert_eq!(info.db(), 3);
        assert_eq!(anon.username(), Some("default"));
    }

    #[test]
    fn noauth_and_wrongpass_map_to_auth() {
        // Act & Assert
        for line in [
            "-NOAUTH Authentication required.\r\n",
            "-WRONGPASS invalid username-password pair or user is disabled.\r\n",
        ] {
            assert_eq!(map_err(server_error(line)).kind, ErrorKind::Auth, "{line}");
        }
    }

    #[test]
    fn server_errors_keep_their_code() {
        // Act
        let e = map_err(server_error("-WRONGTYPE bad type\r\n"));

        // Assert
        assert_eq!(
            e.kind,
            ErrorKind::Query {
                code: Some("WRONGTYPE".into()),
                position: None
            }
        );
        assert_eq!(e.message, "WRONGTYPE bad type");
    }

    #[test]
    fn only_server_replies_count_as_server_errors() {
        // Arrange
        let io = RedisError::from(std::io::Error::from(std::io::ErrorKind::BrokenPipe));

        // Act & Assert
        assert!(is_server_error(&server_error("-NOPERM denied\r\n")));
        assert!(!is_server_error(&io));
    }

    #[test]
    fn ipv6_host_is_bracketed_when_dialled() {
        // Arrange
        let mut t = target();
        t.host = "::1".into();

        // Act & Assert
        assert_eq!(dial_addr(&t), "[::1]:6379");
    }

    #[test]
    fn io_and_protocol_failures_map_to_their_kinds() {
        // Arrange
        let io = |k| RedisError::from(std::io::Error::from(k));
        let odd = RedisError::from((redis::ErrorKind::UnexpectedReturnType, "bad reply"));

        // Act & Assert
        assert_eq!(
            map_err(io(std::io::ErrorKind::TimedOut)).kind,
            ErrorKind::Timeout
        );
        assert_eq!(
            map_err(io(std::io::ErrorKind::ConnectionRefused)).kind,
            ErrorKind::Connect
        );
        assert_eq!(map_err(odd).kind, ErrorKind::Protocol);
    }
}
