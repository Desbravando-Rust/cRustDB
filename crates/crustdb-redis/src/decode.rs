//! Pure decoding of Redis replies into store types.
// Consumed by the session (part B).
#![allow(dead_code)]

use std::time::Duration;

use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Product, ServerVersion};
use crustdb_core::store::value::StreamEntry;
use crustdb_core::store::{Reply, ValueKind};
use redis::Value;

pub(crate) fn to_reply(v: Value) -> Reply {
    match v {
        Value::Nil => Reply::Nil,
        Value::Int(i) => Reply::Int(i),
        Value::Double(d) => Reply::Double(d),
        Value::Boolean(b) => Reply::Bool(b),
        Value::BulkString(b) | Value::BigNumber(b) => Reply::Bulk(b),
        Value::Okay => Reply::Status("OK".to_owned()),
        Value::SimpleString(s) | Value::VerbatimString { text: s, .. } => Reply::Status(s),
        Value::Array(a) | Value::Set(a) | Value::Push { data: a, .. } => {
            Reply::Array(a.into_iter().map(to_reply).collect())
        }
        Value::Map(m) => Reply::Map(
            m.into_iter()
                .map(|(k, v)| (to_reply(k), to_reply(v)))
                .collect(),
        ),
        Value::Attribute { data, .. } => to_reply(*data),
        Value::ServerError(e) => Reply::Status(e.to_string()),
        // `Value` is #[non_exhaustive]
        other => Reply::Status(format!("{other:?}")),
    }
}

/// `None` for `none` (the key is gone).
pub(crate) fn value_kind(t: &str) -> Option<ValueKind> {
    Some(match t {
        "none" => return None,
        "string" => ValueKind::Text,
        "hash" => ValueKind::Map,
        "list" => ValueKind::List,
        "set" => ValueKind::Set,
        "zset" => ValueKind::SortedSet,
        "stream" => ValueKind::Stream,
        other => ValueKind::Other(other.to_owned()),
    })
}

/// Outer `None`: the key vanished (`-2`). Inner `None`: no expiry (`-1`).
pub(crate) fn ttl(pttl: i64) -> Option<Option<Duration>> {
    match pttl {
        -2 => None,
        ..0 => Some(None),
        ms => Some(Some(Duration::from_millis(ms.unsigned_abs()))),
    }
}

/// Callers fetch `limit + 1` elements; only an element past the limit means truncation.
pub(crate) fn cap<T>(mut v: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    let truncated = v.len() > limit;
    v.truncate(limit);
    (v, truncated)
}

fn bytes(v: Value) -> Result<Vec<u8>, DriverError> {
    match v {
        Value::BulkString(b) => Ok(b),
        Value::SimpleString(s) | Value::VerbatimString { text: s, .. } => Ok(s.into_bytes()),
        Value::Int(i) => Ok(i.to_string().into_bytes()),
        Value::Double(d) => Ok(d.to_string().into_bytes()),
        _ => Err(protocol("expected a string in the reply")),
    }
}

/// RESP2 flat `[k, v, k, v]` or RESP3 map.
fn flat_pairs(v: Value) -> Result<Vec<(Value, Value)>, DriverError> {
    match v {
        Value::Map(m) => Ok(m),
        Value::Nil => Ok(Vec::new()),
        Value::Array(a) if a.len() % 2 == 0 => {
            let mut it = a.into_iter();
            let mut out = Vec::with_capacity(it.len() / 2);
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                out.push((k, v));
            }
            Ok(out)
        }
        _ => Err(protocol("expected key/value pairs in the reply")),
    }
}

type BytePairs = Vec<(Vec<u8>, Vec<u8>)>;

pub(crate) fn pairs(v: Value) -> Result<BytePairs, DriverError> {
    flat_pairs(v)?
        .into_iter()
        .map(|(k, v)| Ok((bytes(k)?, bytes(v)?)))
        .collect()
}

fn score(v: Value) -> Result<f64, DriverError> {
    match v {
        Value::Double(d) => Ok(d),
        other => String::from_utf8_lossy(&bytes(other)?)
            .parse()
            .map_err(|_| protocol("invalid zset score")),
    }
}

/// `ZRANGE .. WITHSCORES`: flat in RESP2, `[member, score]` pairs in RESP3.
pub(crate) fn zset_pairs(v: Value) -> Result<Vec<(Vec<u8>, f64)>, DriverError> {
    let Value::Array(items) = v else {
        return Err(protocol("expected an array for a sorted set"));
    };
    if matches!(items.first(), Some(Value::Array(_))) {
        return items
            .into_iter()
            .map(|p| match p {
                Value::Array(mut p) if p.len() == 2 => {
                    let s = score(p.remove(1))?;
                    Ok((bytes(p.remove(0))?, s))
                }
                _ => Err(protocol("malformed sorted set entry")),
            })
            .collect();
    }
    flat_pairs(Value::Array(items))?
        .into_iter()
        .map(|(m, s)| Ok((bytes(m)?, score(s)?)))
        .collect()
}

/// `XRANGE` reply: `[[id, [field, value, ...]], ...]`.
pub(crate) fn stream_entries(v: Value) -> Result<Vec<StreamEntry>, DriverError> {
    let Value::Array(entries) = v else {
        return Err(protocol("expected an array for a stream"));
    };
    entries
        .into_iter()
        .map(|e| match e {
            Value::Array(mut e) if e.len() == 2 => {
                let fields = pairs(e.remove(1))?;
                let id = String::from_utf8_lossy(&bytes(e.remove(0))?).into_owned();
                Ok(StreamEntry { id, fields })
            }
            _ => Err(protocol("malformed stream entry")),
        })
        .collect()
}

pub(crate) fn parse_info_version(info: &str) -> ServerVersion {
    let raw = info
        .lines()
        .find_map(|l| l.trim().strip_prefix("redis_version:"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    let mut parts = raw.split('.').map(|p| {
        p.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    });
    ServerVersion {
        product: Product::Redis,
        major: parts.next().unwrap_or(0),
        minor: parts.next().unwrap_or(0),
        patch: parts.next().unwrap_or(0),
        raw,
    }
}

/// `CONFIG GET databases` -> `"0"..N`; `None` when the server hides it (ACL).
pub(crate) fn namespaces_from_config(v: Value) -> Option<Vec<String>> {
    let (_, n) = flat_pairs(v).ok()?.into_iter().next()?;
    let n: u32 = String::from_utf8_lossy(&bytes(n).ok()?).parse().ok()?;
    Some((0..n).map(|i| i.to_string()).collect())
}

/// Fallback from `INFO keyspace`; the current db is listed even when empty.
pub(crate) fn namespaces_from_keyspace(info: &str, current: u32) -> Vec<String> {
    let mut dbs: Vec<u32> = info
        .lines()
        .filter_map(|l| l.strip_prefix("db")?.split(':').next()?.parse().ok())
        .collect();
    dbs.push(current);
    dbs.sort_unstable();
    dbs.dedup();
    dbs.iter().map(u32::to_string).collect()
}

fn protocol(message: &str) -> DriverError {
    DriverError {
        kind: ErrorKind::Protocol,
        message: message.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bulk(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }

    #[test]
    fn sentinel_marks_truncated_only_when_over_limit() {
        // Act & Assert
        assert_eq!(cap(vec![1, 2, 3], 3), (vec![1, 2, 3], false));
        assert_eq!(cap(vec![1, 2, 3, 4], 3), (vec![1, 2, 3], true));
        assert_eq!(cap(Vec::<i32>::new(), 3), (vec![], false));
    }

    #[test]
    fn pttl_minus_one_means_no_expiry() {
        // Act & Assert
        assert_eq!(ttl(-1), Some(None));
        assert_eq!(ttl(1500), Some(Some(Duration::from_millis(1500))));
    }

    #[test]
    fn vanished_key_is_skipped() {
        // Act & Assert
        assert_eq!(ttl(-2), None);
    }

    #[test]
    fn unknown_type_maps_to_other_kind() {
        // Act & Assert
        assert_eq!(value_kind("none"), None);
        assert_eq!(value_kind("string"), Some(ValueKind::Text));
        assert_eq!(value_kind("zset"), Some(ValueKind::SortedSet));
        assert_eq!(
            value_kind("ReJSON-RL"),
            Some(ValueKind::Other("ReJSON-RL".into()))
        );
    }

    #[test]
    fn zset_reply_keeps_score_order_and_parses_inf() {
        // Arrange
        let resp2 = Value::Array(vec![
            bulk("a"),
            bulk("-inf"),
            bulk("b"),
            bulk("1.5"),
            bulk("c"),
            bulk("inf"),
        ]);
        let resp3 = Value::Array(vec![
            Value::Array(vec![bulk("a"), Value::Double(2.0)]),
            Value::Array(vec![bulk("b"), Value::Double(3.0)]),
        ]);

        // Act
        let flat = zset_pairs(resp2).unwrap();
        let nested = zset_pairs(resp3).unwrap();

        // Assert
        assert_eq!(
            flat,
            vec![
                (b"a".to_vec(), f64::NEG_INFINITY),
                (b"b".to_vec(), 1.5),
                (b"c".to_vec(), f64::INFINITY)
            ]
        );
        assert_eq!(nested, vec![(b"a".to_vec(), 2.0), (b"b".to_vec(), 3.0)]);
    }

    #[test]
    fn hash_reply_accepts_flat_and_map_forms() {
        // Arrange
        let flat = Value::Array(vec![bulk("f"), bulk("v")]);
        let map = Value::Map(vec![(bulk("f"), bulk("v"))]);

        // Act & Assert
        let expected = vec![(b"f".to_vec(), b"v".to_vec())];
        assert_eq!(pairs(flat).unwrap(), expected);
        assert_eq!(pairs(map).unwrap(), expected);
    }

    #[test]
    fn stream_reply_maps_ids_and_fields() {
        // Arrange
        let reply = Value::Array(vec![
            Value::Array(vec![
                bulk("1-0"),
                Value::Array(vec![bulk("a"), bulk("1"), bulk("b"), bulk("2")]),
            ]),
            Value::Array(vec![bulk("2-0"), Value::Array(vec![bulk("c"), bulk("3")])]),
        ]);

        // Act
        let entries = stream_entries(reply).unwrap();

        // Assert
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "1-0");
        assert_eq!(
            entries[0].fields,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec())
            ]
        );
        assert_eq!(entries[1].id, "2-0");
    }

    #[test]
    fn status_like_values_map_to_status_reply() {
        // Act & Assert
        assert_eq!(to_reply(Value::Okay), Reply::Status("OK".into()));
        assert_eq!(
            to_reply(Value::SimpleString("PONG".into())),
            Reply::Status("PONG".into())
        );
        assert_eq!(
            to_reply(Value::Set(vec![Value::Int(1)])),
            Reply::Array(vec![Reply::Int(1)])
        );
        assert_eq!(
            to_reply(Value::Attribute {
                data: Box::new(Value::Int(7)),
                attributes: vec![],
            }),
            Reply::Int(7)
        );
        assert_eq!(to_reply(Value::Nil), Reply::Nil);
    }

    #[test]
    fn server_error_reply_keeps_its_text() {
        // Arrange
        let value = redis::parse_redis_value(b"-ERR boom\r\n").unwrap();

        // Act
        let reply = to_reply(value);

        // Assert
        assert!(matches!(reply, Reply::Status(s) if s.contains("boom")));
    }

    #[test]
    fn parses_redis_version_from_info_server() {
        // Arrange
        let info = "# Server\r\nredis_mode:standalone\r\nredis_version:7.2.4\r\nos:Linux\r\n";

        // Act
        let v = parse_info_version(info);

        // Assert
        assert_eq!(v.product, Product::Redis);
        assert_eq!((v.major, v.minor, v.patch), (7, 2, 4));
        assert_eq!(v.raw, "7.2.4");
    }

    #[test]
    fn config_databases_lists_all_indexes() {
        // Arrange
        let flat = Value::Array(vec![bulk("databases"), bulk("3")]);
        let map = Value::Map(vec![(bulk("databases"), bulk("3"))]);

        // Act & Assert
        let expected = Some(vec!["0".to_owned(), "1".into(), "2".into()]);
        assert_eq!(namespaces_from_config(flat), expected);
        assert_eq!(namespaces_from_config(map), expected);
        assert_eq!(namespaces_from_config(Value::Array(vec![])), None);
    }

    #[test]
    fn keyspace_fallback_includes_current_empty_db() {
        // Arrange
        let info = "# Keyspace\r\ndb0:keys=3,expires=0,avg_ttl=0\r\ndb10:keys=1,expires=0\r\n";

        // Act
        let ns = namespaces_from_keyspace(info, 5);

        // Assert
        assert_eq!(ns, vec!["0", "5", "10"]);
    }
}
