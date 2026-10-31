use std::path::PathBuf;

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
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
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
        "redis" | "rediss" => DriverKind::Redis,
        "sqlite" => return sqlite(&url),
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

    let query_path = |names: [&str; 2]| {
        url.query_pairs()
            .find(|(k, v)| names.contains(&k.as_ref()) && !v.is_empty())
            .map(|(_, v)| PathBuf::from(v.into_owned()))
    };
    let client_cert_path = query_path(["sslcert", "ssl-cert"]);
    let client_key_path = query_path(["sslkey", "ssl-key"]);

    let mut parsed = ParsedUrl {
        driver,
        host,
        port: url.port(),
        user,
        password,
        database,
        tls_mode,
        client_cert_path,
        client_key_path,
        warnings,
    };
    if driver == DriverKind::Redis {
        redis_rules(&mut parsed, url.scheme() == "rediss")?;
    }
    Ok(parsed)
}

/// Redis specifics: ACL user defaults to `default`, the path is a numeric db index,
/// and TLS follows the scheme (`redis` plain, `rediss` verified unless `sslmode=require`).
fn redis_rules(p: &mut ParsedUrl, tls: bool) -> Result<(), UrlError> {
    p.user.get_or_insert_with(|| "default".to_owned());
    if p.database
        .as_ref()
        .is_some_and(|d| !d.bytes().all(|b| b.is_ascii_digit()) || d.parse::<u32>().is_err())
    {
        return Err(UrlError::Malformed);
    }
    if tls {
        // VerifyCa is not offered here: the driver cannot verify the chain without the name.
        if p.tls_mode != Some(TlsMode::Require) {
            p.tls_mode = Some(TlsMode::VerifyFull);
        }
    } else {
        p.tls_mode = Some(TlsMode::Disable);
        p.warnings.clear();
    }
    Ok(())
}

/// `sqlite:///abs/path.db`: the decoded path is the database file; there is no host
/// (only `localhost` is accepted, as in `file://`).
fn sqlite(url: &::url::Url) -> Result<ParsedUrl, UrlError> {
    if url
        .host_str()
        .is_some_and(|h| !h.is_empty() && h != "localhost")
    {
        return Err(UrlError::Malformed);
    }
    let path = percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|_| UrlError::Malformed)?;
    Ok(ParsedUrl {
        driver: DriverKind::Sqlite,
        host: String::new(),
        port: None,
        user: None,
        password: None,
        database: Some(path.into_owned()).filter(|d| !d.is_empty()),
        tls_mode: None,
        client_cert_path: None,
        client_key_path: None,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn parses_postgres_with_sslmode() {
        // Act
        let p = parse_connection_url("postgres://u:p%40ss@h:6543/db?sslmode=verify-full").unwrap();

        // Assert
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
    fn parses_postgres_sslcert_sslkey() {
        // Act
        let p = parse_connection_url("postgres://h/db?sslcert=/c/client.crt&sslkey=/c/client.key")
            .unwrap();

        // Assert
        assert_eq!(p.client_cert_path, Some(PathBuf::from("/c/client.crt")));
        assert_eq!(p.client_key_path, Some(PathBuf::from("/c/client.key")));
    }

    #[test]
    fn parses_mysql_ssl_cert_ssl_key() {
        // Act
        let p = parse_connection_url("mysql://h/db?ssl-cert=/c/client.crt&ssl-key=/c/client.key")
            .unwrap();

        // Assert
        assert_eq!(p.client_cert_path, Some(PathBuf::from("/c/client.crt")));
        assert_eq!(p.client_key_path, Some(PathBuf::from("/c/client.key")));
    }

    #[test]
    fn empty_cert_and_key_are_ignored() {
        // Act
        let p = parse_connection_url("postgres://h/db?sslcert=&sslkey=").unwrap();

        // Assert
        assert_eq!((p.client_cert_path, p.client_key_path), (None, None));
    }

    #[test]
    fn percent_decoded_cert_path() {
        // Act
        let p = parse_connection_url("postgres://h/db?sslcert=/my%20certs/c.crt").unwrap();

        // Assert
        assert_eq!(p.client_cert_path, Some(PathBuf::from("/my certs/c.crt")));
        assert_eq!(p.client_key_path, None);
    }

    #[test]
    fn postgresql_scheme_accepted() {
        // Act
        let p = parse_connection_url("postgresql://h").unwrap();

        // Assert
        assert_eq!(p.driver, DriverKind::Postgres);
        assert_eq!(
            (p.port, p.user, p.database, p.tls_mode),
            (None, None, None, None)
        );
    }

    #[test]
    fn mariadb_maps_to_mysql() {
        // Act & Assert
        assert_eq!(
            parse_connection_url("mariadb://h").unwrap().driver,
            DriverKind::Mysql
        );
    }

    #[test]
    fn mysql_ssl_mode_verify_identity_maps_to_verify_full() {
        // Act
        let p = parse_connection_url("mysql://h/db?ssl-mode=VERIFY_IDENTITY").unwrap();

        // Assert
        assert_eq!(p.tls_mode, Some(TlsMode::VerifyFull));
    }

    #[test]
    fn prefer_downgrades_to_require_with_warning() {
        // Act & Assert
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
    fn parses_sqlite_url_as_file_path() {
        // Act & Assert
        let p = parse_connection_url("sqlite:///tmp/a%20b.db").unwrap();
        assert_eq!(p.driver, DriverKind::Sqlite);
        assert_eq!(p.database.as_deref(), Some("/tmp/a b.db"));
        assert_eq!(p.host, "");
        let p = parse_connection_url("sqlite:///tmp/a.db").unwrap();
        assert_eq!(p.database.as_deref(), Some("/tmp/a.db"));
        let p = parse_connection_url("sqlite://localhost/tmp/a.db").unwrap();
        assert_eq!(p.database.as_deref(), Some("/tmp/a.db"));
    }

    #[test]
    fn sqlite_url_with_host_rejected() {
        // Act & Assert
        // `dados.db` would be the host, silently leaving no file.
        assert_eq!(
            parse_connection_url("sqlite://dados.db").unwrap_err(),
            UrlError::Malformed
        );
    }

    #[test]
    fn unknown_scheme_rejected() {
        // Act & Assert
        assert_eq!(
            parse_connection_url("mongodb://h").unwrap_err(),
            UrlError::UnsupportedScheme("mongodb".into())
        );
    }

    #[test]
    fn parses_rediss_with_acl_user_and_db() {
        // Act
        let p = parse_connection_url("rediss://alice:p%40ss@h:6380/3").unwrap();

        // Assert
        assert_eq!(p.driver, DriverKind::Redis);
        assert_eq!(p.user.as_deref(), Some("alice"));
        assert_eq!(p.password.as_ref().unwrap().expose_secret(), "p@ss");
        assert_eq!(p.port, Some(6380));
        assert_eq!(p.database.as_deref(), Some("3"));
        assert_eq!(p.tls_mode, Some(TlsMode::VerifyFull));
    }

    #[test]
    fn redis_password_only_url_uses_default_user() {
        // Act
        let p = parse_connection_url("redis://:p@h").unwrap();

        // Assert
        assert_eq!(p.user.as_deref(), Some("default"));
        assert_eq!(p.password.as_ref().unwrap().expose_secret(), "p");
    }

    #[test]
    fn redis_db_path_must_be_number() {
        // Act & Assert
        for u in [
            "redis://h/abc",
            "redis://h/-1",
            "redis://h/+3",
            "redis://h/3/4",
            "redis://h/4294967296",
        ] {
            assert_eq!(
                parse_connection_url(u).unwrap_err(),
                UrlError::Malformed,
                "{u}"
            );
        }
        assert_eq!(parse_connection_url("redis://h/").unwrap().database, None);
    }

    #[test]
    fn redis_verify_ca_is_treated_as_verify_full() {
        // Act
        let p = parse_connection_url("rediss://h?sslmode=verify-ca").unwrap();

        // Assert
        assert_eq!(p.tls_mode, Some(TlsMode::VerifyFull));
    }

    #[test]
    fn redis_require_skips_verification() {
        // Act
        let p = parse_connection_url("rediss://h?sslmode=require").unwrap();

        // Assert
        assert_eq!(p.tls_mode, Some(TlsMode::Require));
    }

    #[test]
    fn plain_redis_url_disables_tls() {
        // Act
        let p = parse_connection_url("redis://h?sslmode=prefer").unwrap();

        // Assert
        assert_eq!(p.tls_mode, Some(TlsMode::Disable));
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn ipv6_host_parsed() {
        // Act
        let p = parse_connection_url("postgres://[::1]:5432/db").unwrap();

        // Assert
        assert_eq!(p.host, "::1");
        assert_eq!(p.port, Some(5432));
    }

    #[test]
    fn errors_and_ignored_values() {
        // Act & Assert
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
        // Act
        let p = parse_connection_url("postgres://u:hunter2@h/db").unwrap();

        // Assert
        assert!(!format!("{p:?}").contains("hunter2"));
    }
}
