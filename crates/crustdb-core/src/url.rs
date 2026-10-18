use percent_encoding::percent_decode_str;
use secrecy::SecretString;

use crate::driver::TlsMode;
use crate::model::DriverKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlWarning {
    DowngradedToRequire(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("missing host")]
    MissingHost,
    #[error("invalid port")]
    InvalidPort,
    #[error("malformed URL")]
    Malformed,
}

#[derive(Debug, Clone)]
pub struct ParsedUrl {
    pub driver: DriverKind,
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub database: Option<String>,
    pub tls_mode: Option<TlsMode>,
    pub warnings: Vec<UrlWarning>,
}

pub fn parse_connection_url(s: &str) -> Result<ParsedUrl, UrlError> {
    let url = ::url::Url::parse(s).map_err(|e| match e {
        ::url::ParseError::InvalidPort => UrlError::InvalidPort,
        ::url::ParseError::EmptyHost => UrlError::MissingHost,
        _ => UrlError::Malformed,
    })?;
    let driver = match url.scheme() {
        "postgres" | "postgresql" => DriverKind::Postgres,
        "mysql" | "mariadb" => DriverKind::Mysql,
        other => return Err(UrlError::UnsupportedScheme(other.to_owned())),
    };
    let host = url
        .host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
        .ok_or(UrlError::MissingHost)?
        .to_owned();
    let decode = |v: &str| {
        percent_decode_str(v)
            .decode_utf8()
            .map(|c| c.into_owned())
            .map_err(|_| UrlError::Malformed)
    };
    let user = Some(decode(url.username())?).filter(|u| !u.is_empty());
    let password = url
        .password()
        .map(|p| decode(p).map(SecretString::from))
        .transpose()?;
    let database = Some(decode(url.path().trim_start_matches('/'))?).filter(|d| !d.is_empty());

    let mut tls_mode = None;
    let mut warnings = Vec::new();
    let tls_value = url
        .query_pairs()
        .find(|(k, _)| k == "sslmode" || k == "ssl-mode")
        .map(|(_, v)| v.into_owned());
    if let Some(v) = tls_value {
        tls_mode = match v.to_ascii_lowercase().as_str() {
            "disable" | "disabled" => Some(TlsMode::Disable),
            "allow" | "prefer" | "preferred" => {
                warnings.push(UrlWarning::DowngradedToRequire(v));
                Some(TlsMode::Require)
            }
            "require" | "required" => Some(TlsMode::Require),
            "verify-ca" | "verify_ca" => Some(TlsMode::VerifyCa),
            "verify-full" | "verify_identity" => Some(TlsMode::VerifyFull),
            _ => None,
        };
    }

    Ok(ParsedUrl {
        driver,
        host,
        port: url.port(),
        user,
        password,
        database,
        tls_mode,
        warnings,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn parses_postgres_with_sslmode() {
        let p = parse_connection_url("postgres://u:p%40ss@h:6543/db?sslmode=verify-full").unwrap();
        assert_eq!(p.driver, DriverKind::Postgres);
        assert_eq!(p.host, "h");
        assert_eq!(p.port, Some(6543));
        assert_eq!(p.user.as_deref(), Some("u"));
        assert_eq!(p.password.as_ref().unwrap().expose_secret(), "p@ss");
        assert_eq!(p.database.as_deref(), Some("db"));
        assert_eq!(p.tls_mode, Some(TlsMode::VerifyFull));
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn postgresql_scheme_accepted() {
        let p = parse_connection_url("postgresql://h").unwrap();
        assert_eq!(p.driver, DriverKind::Postgres);
        assert_eq!(
            (p.port, p.user, p.database, p.tls_mode),
            (None, None, None, None)
        );
    }

    #[test]
    fn mariadb_maps_to_mysql() {
        assert_eq!(
            parse_connection_url("mariadb://h").unwrap().driver,
            DriverKind::Mysql
        );
    }

    #[test]
    fn mysql_ssl_mode_verify_identity_maps_to_verify_full() {
        let p = parse_connection_url("mysql://h/db?ssl-mode=VERIFY_IDENTITY").unwrap();
        assert_eq!(p.tls_mode, Some(TlsMode::VerifyFull));
    }

    #[test]
    fn prefer_downgrades_to_require_with_warning() {
        let p = parse_connection_url("postgres://h/db?sslmode=prefer").unwrap();
        assert_eq!(p.tls_mode, Some(TlsMode::Require));
        assert_eq!(
            p.warnings,
            vec![UrlWarning::DowngradedToRequire("prefer".into())]
        );
        let p = parse_connection_url("mysql://h/db?ssl-mode=PREFERRED").unwrap();
        assert_eq!(
            p.warnings,
            vec![UrlWarning::DowngradedToRequire("PREFERRED".into())]
        );
    }

    #[test]
    fn unknown_scheme_rejected() {
        assert_eq!(
            parse_connection_url("redis://h").unwrap_err(),
            UrlError::UnsupportedScheme("redis".into())
        );
    }

    #[test]
    fn ipv6_host_parsed() {
        let p = parse_connection_url("postgres://[::1]:5432/db").unwrap();
        assert_eq!(p.host, "::1");
        assert_eq!(p.port, Some(5432));
    }

    #[test]
    fn errors_and_ignored_values() {
        assert_eq!(
            parse_connection_url("postgres:///db").unwrap_err(),
            UrlError::MissingHost
        );
        assert_eq!(
            parse_connection_url("postgres://h:99999/db").unwrap_err(),
            UrlError::InvalidPort
        );
        assert_eq!(
            parse_connection_url("not a url").unwrap_err(),
            UrlError::Malformed
        );
        for u in [
            "postgres://u@/db",
            "postgres://u:p@:5432/db",
            "postgres://:5432/db",
        ] {
            assert_eq!(
                parse_connection_url(u).unwrap_err(),
                UrlError::MissingHost,
                "{u}"
            );
        }
        assert_eq!(
            parse_connection_url("postgres://u:%FF@h/db").unwrap_err(),
            UrlError::Malformed
        );
        let p = parse_connection_url("postgres://h/db?sslmode=bogus&x=1").unwrap();
        assert_eq!(p.tls_mode, None);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn debug_does_not_leak_password() {
        let p = parse_connection_url("postgres://u:hunter2@h/db").unwrap();
        assert!(!format!("{p:?}").contains("hunter2"));
    }
}
