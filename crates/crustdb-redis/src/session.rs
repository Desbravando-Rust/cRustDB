//! A live Redis connection and its `StoreSession` implementation.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use crustdb_core::driver::SessionPurpose;
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{CELL_MEMORY_CAP, ServerVersion};
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::value::ValueBody;
use crustdb_core::store::{
    EntryView, KeyEntry, Reply, ScanPage, ScanRequest, StoreSession, ValueKind,
};
use redis::Value;
use redis::aio::MultiplexedConnection;
use tokio::task::JoinHandle;

use crate::classify::guard_run;
use crate::commands::{build, scan_argv};
use crate::connect::{from_value, is_server_error, map_err};
use crate::decode;
use crate::tls::err;

pub(crate) struct RedisSession {
    conn: MultiplexedConnection,
    /// The connection's I/O loop. Dropping the connection alone does not close the socket while
    /// a command is in flight, so the loop is aborted explicitly.
    driver: JoinHandle<()>,
    purpose: SessionPurpose,
    version: ServerVersion,
    db: u32,
}

impl RedisSession {
    pub(crate) fn new(
        conn: MultiplexedConnection,
        driver: JoinHandle<()>,
        purpose: SessionPurpose,
        db: u32,
    ) -> Self {
        Self {
            conn,
            driver,
            purpose,
            version: decode::parse_info_version(""),
            db,
        }
    }

    pub(crate) fn set_version(&mut self, version: ServerVersion) {
        self.version = version;
    }

    /// Runs one command given as byte arguments.
    pub(crate) async fn call<I, A>(&mut self, argv: I) -> Result<Value, DriverError>
    where
        I: IntoIterator<Item = A>,
        A: AsRef<[u8]>,
    {
        build(argv)
            .query_async(&mut self.conn)
            .await
            .map_err(map_err)
    }

    /// `(TYPE, PTTL)` of each key in one round trip. A key the server refuses to inspect (ACL
    /// checks key patterns for `TYPE`/`PTTL` but not for `SCAN`) yields `Err` for that key only.
    async fn types_and_ttls(
        &mut self,
        keys: &[Vec<u8>],
    ) -> Result<Vec<Result<(String, i64), DriverError>>, DriverError> {
        // An empty pipeline is rejected client-side ("empty command").
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        for k in keys {
            pipe.add_command(build([b"TYPE", k.as_slice()]));
            pipe.add_command(build([b"PTTL", k.as_slice()]));
        }
        let flat: Vec<Value> = pipe.query_async(&mut self.conn).await.map_err(map_err)?;
        if flat.len() != keys.len() * 2 {
            return Err(err(
                ErrorKind::Protocol,
                "malformed TYPE/PTTL pipeline reply",
            ));
        }
        flat.as_chunks::<2>()
            .0
            .iter()
            .map(|[ty, ms]| match (ty, ms) {
                (Value::ServerError(e), _) | (_, Value::ServerError(e)) => {
                    Ok(Err(map_err(e.clone().into())))
                }
                _ => Ok(Ok((from_value(ty.clone())?, from_value(ms.clone())?))),
            })
            .collect()
    }

    /// Accumulates `HSCAN`/`SSCAN` items until `limit + 1` elements (`per_item` replies each,
    /// 2 for hashes) are collected or the cursor wraps.
    async fn sub_scan(
        &mut self,
        name: &str,
        key: &[u8],
        limit: usize,
        per_item: usize,
    ) -> Result<Vec<Value>, DriverError> {
        let wanted = (limit + 1) * per_item;
        let (mut cursor, mut items) = (String::from("0"), Vec::new());
        loop {
            let argv = scan_argv(name, Some(key), &cursor, None, limit + 1);
            let reply = self.call(argv).await?;
            let (next, page) = from_value::<(String, Vec<Value>)>(reply)?;
            items.extend(page);
            if scan_done(&next, items.len(), wanted) {
                return Ok(items);
            }
            cursor = next;
        }
    }

    async fn body(
        &mut self,
        key: &[u8],
        kind: &ValueKind,
        limit: usize,
    ) -> Result<(ValueBody, bool), DriverError> {
        let to_arg = |n: usize| n.to_string().into_bytes();
        Ok(match kind {
            ValueKind::Text => {
                let end = to_arg(CELL_MEMORY_CAP);
                let v = self.call([b"GETRANGE", key, b"0", &end]).await?;
                let (bytes, t) = decode::cap(from_value::<Vec<u8>>(v)?, CELL_MEMORY_CAP);
                (ValueBody::Text(bytes), t)
            }
            ValueKind::Map => {
                let items = self.sub_scan("HSCAN", key, limit, 2).await?;
                let (v, t) = decode::cap(decode::pairs(Value::Array(items))?, limit);
                (ValueBody::Map(v), t)
            }
            ValueKind::Set => {
                let items = self.sub_scan("SSCAN", key, limit, 1).await?;
                let members = from_value::<Vec<Vec<u8>>>(Value::Array(items))?;
                let (v, t) = decode::cap(members, limit);
                (ValueBody::Set(v), t)
            }
            ValueKind::SortedSet => {
                let last = to_arg(limit);
                let v = self
                    .call([&b"ZRANGE"[..], key, b"0", &last, b"WITHSCORES"])
                    .await?;
                let (v, t) = decode::cap(decode::zset_pairs(v)?, limit);
                (ValueBody::SortedSet(v), t)
            }
            ValueKind::List => {
                let last = to_arg(limit);
                let v = self.call([b"LRANGE", key, b"0", &last]).await?;
                let items = from_value::<Vec<Vec<u8>>>(v)?;
                let (v, t) = decode::cap(items, limit);
                (ValueBody::List(v), t)
            }
            ValueKind::Stream => {
                let count = to_arg(limit + 1);
                let v = self
                    .call([&b"XRANGE"[..], key, b"-", b"+", b"COUNT", &count])
                    .await?;
                let (v, t) = decode::cap(decode::stream_entries(v)?, limit);
                (ValueBody::Stream(v), t)
            }
            ValueKind::Other(t) => {
                return Err(err(
                    ErrorKind::Unsupported,
                    format!("cannot display values of type `{t}`"),
                ));
            }
        })
    }

    /// `None` when the server refuses or cannot compute it; I/O errors still fail.
    async fn memory_usage(&mut self, key: &[u8]) -> Result<Option<u64>, DriverError> {
        match build([&b"MEMORY"[..], b"USAGE", key])
            .query_async::<Option<u64>>(&mut self.conn)
            .await
        {
            Ok(n) => Ok(n),
            Err(e) if is_server_error(&e) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    async fn keyspace_namespaces(&mut self) -> Result<Vec<String>, DriverError> {
        let info = self.call(["INFO", "keyspace"]).await?;
        let info = from_value::<String>(info)?;
        Ok(decode::namespaces_from_keyspace(&info, self.db))
    }
}

/// Pairs scanned keys with their `(TYPE, PTTL)`. Keys that vanished (expired between `SCAN` and
/// `TYPE`) or that the server refused to inspect are dropped.
fn key_entries(
    keys: Vec<Vec<u8>>,
    infos: Vec<Result<(String, i64), DriverError>>,
) -> Vec<KeyEntry> {
    keys.into_iter()
        .zip(infos)
        .filter_map(|(key, info)| {
            let (ty, pttl) = info.ok()?;
            Some(KeyEntry {
                key: EntryKey(key),
                kind: decode::value_kind(&ty)?,
                ttl: decode::ttl(pttl)?,
            })
        })
        .collect()
}

/// Only server-side errors (ACL denied, renamed command) fall back; I/O and auth failures propagate.
fn falls_back(kind: &ErrorKind) -> bool {
    matches!(kind, ErrorKind::Query { .. })
}

fn scan_done(next_cursor: &str, collected: usize, wanted: usize) -> bool {
    next_cursor == "0" || collected >= wanted
}

impl Drop for RedisSession {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

#[async_trait]
impl StoreSession for RedisSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }

    async fn ping(&mut self) -> Result<Duration, DriverError> {
        let start = Instant::now();
        self.call(["PING"]).await?;
        Ok(start.elapsed())
    }

    async fn namespaces(&mut self) -> Result<Vec<String>, DriverError> {
        match self.call(["CONFIG", "GET", "databases"]).await {
            Ok(v) => match decode::namespaces_from_config(v) {
                Some(n) => Ok(n),
                None => self.keyspace_namespaces().await,
            },
            // ACLs often forbid CONFIG; I/O errors are not a reason to fall back.
            Err(e) if falls_back(&e.kind) => self.keyspace_namespaces().await,
            Err(e) => Err(e),
        }
    }

    async fn scan(&mut self, req: &ScanRequest) -> Result<ScanPage, DriverError> {
        let cursor = req.cursor.as_deref().unwrap_or("0");
        let argv = scan_argv(
            "SCAN",
            None,
            cursor,
            req.pattern.as_deref(),
            req.count as usize,
        );
        let reply = self.call(argv).await?;
        let (next, keys) = from_value::<(String, Vec<Vec<u8>>)>(reply)?;
        let infos = self.types_and_ttls(&keys).await?;
        let entries = key_entries(keys, infos);
        Ok(ScanPage {
            entries,
            next: (next != "0").then_some(next),
        })
    }

    async fn read(&mut self, key: &EntryKey, limit: usize) -> Result<EntryView, DriverError> {
        let (ty, pttl) = self
            .types_and_ttls(std::slice::from_ref(&key.0))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| err(ErrorKind::Protocol, "missing TYPE/PTTL reply"))??;
        let gone = || {
            err(
                ErrorKind::Query {
                    code: None,
                    position: None,
                },
                "key no longer exists",
            )
        };
        let kind = decode::value_kind(&ty).ok_or_else(gone)?;
        let ttl = decode::ttl(pttl).ok_or_else(gone)?;
        let (body, truncated) = self.body(&key.0, &kind, limit).await?;
        let size_bytes = self.memory_usage(&key.0).await?;
        Ok(EntryView {
            ttl,
            size_bytes,
            body,
            truncated,
        })
    }

    async fn run(&mut self, argv: &[Vec<u8>]) -> Result<Reply, DriverError> {
        guard_run(self.purpose, argv)?;
        Ok(decode::to_reply(self.call(argv).await?))
    }

    async fn close(self: Box<Self>) {}
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::{Duration, Instant};

    use crustdb_core::driver::{ConnectTarget, TlsConfig, TlsMode};
    use secrecy::SecretString;

    use super::*;

    fn denied() -> DriverError {
        err(
            ErrorKind::Query {
                code: Some("NOPERM".into()),
                position: None,
            },
            "NOPERM",
        )
    }

    #[test]
    fn scan_page_keeps_only_keys_that_can_be_inspected() {
        // Arrange
        let keys = vec![
            b"ok".to_vec(),
            b"gone".to_vec(),
            b"denied".to_vec(),
            b"mod".to_vec(),
        ];
        let infos = vec![
            Ok(("string".to_owned(), 1500)),
            Ok(("none".to_owned(), -2)),
            Err(denied()),
            Ok(("ReJSON-RL".to_owned(), -1)),
        ];

        // Act
        let entries = key_entries(keys, infos);

        // Assert
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, EntryKey(b"ok".to_vec()));
        assert_eq!(entries[0].kind, ValueKind::Text);
        assert_eq!(entries[0].ttl, Some(Duration::from_millis(1500)));
        assert_eq!(entries[1].kind, ValueKind::Other("ReJSON-RL".into()));
        assert_eq!(entries[1].ttl, None);
    }

    #[test]
    fn only_server_errors_fall_back_for_namespaces() {
        // Act & Assert
        assert!(falls_back(&denied().kind));
        for kind in [ErrorKind::Connect, ErrorKind::Auth, ErrorKind::Timeout] {
            assert!(!falls_back(&kind), "{kind:?}");
        }
    }

    #[test]
    fn sub_scan_stops_at_cursor_wrap_or_enough_items() {
        // Act & Assert
        assert!(scan_done("0", 0, 10));
        assert!(scan_done("17", 10, 10));
        assert!(!scan_done("17", 9, 10));
    }

    async fn open(redis: &crustdb_testkit::RedisContainer) -> RedisSession {
        let t = ConnectTarget {
            host: redis.host.clone(),
            port: redis.port,
            connect_addr: None,
            user: String::new(),
            password: Some(SecretString::from(redis.password.clone())),
            database: None,
            tls: TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
                client_cert_path: None,
                client_key_path: None,
            },
            connect_timeout: Duration::from_secs(10),
        };
        crate::connect::connect(&t, SessionPurpose::Editor)
            .await
            .unwrap()
    }

    /// Needs the crate-internal `call`: `run` refuses blocking commands such as `WAIT`, and a
    /// command that blocks the whole server (`DEBUG SLEEP`) would also stall the observer.
    #[tokio::test]
    #[ignore = "docker"]
    async fn dropping_session_with_command_in_flight_closes_client() {
        // Arrange
        let redis = crustdb_testkit::start_redis(None).await;
        let mut observer = open(&redis).await;
        let mut busy = open(&redis).await;
        let id: i64 = from_value(busy.call(["CLIENT", "ID"]).await.unwrap()).unwrap();

        // Act: WAIT blocks this client for 5 s; abandon it mid-flight.
        let in_flight =
            tokio::time::timeout(Duration::from_millis(200), busy.call(["WAIT", "1", "5000"]))
                .await;
        drop(busy);

        // Assert: the client vanishes well before the 5 s WAIT would end.
        assert!(in_flight.is_err(), "WAIT must still be running");
        let marker = format!("id={id} ");
        let start = Instant::now();
        loop {
            let list: String =
                from_value(observer.call(["CLIENT", "LIST"]).await.unwrap()).unwrap();
            if !list.contains(&marker) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "client {id} still connected"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
