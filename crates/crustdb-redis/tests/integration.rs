//! Docker integration tests: `cargo test -p crustdb-redis -- --include-ignored`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn StoreSession> is not Debug

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crustdb_core::driver::{ConnectTarget, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::Product;
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::value::ValueBody;
use crustdb_core::store::{Reply, ScanRequest, StoreDriver, StoreSession};
use crustdb_redis::RedisDriver;
use crustdb_testkit::{RedisContainer, certs_for, start_redis};
use secrecy::SecretString;

fn target(c: &RedisContainer, user: &str, password: &str, db: Option<&str>) -> ConnectTarget {
    ConnectTarget {
        host: c.host.clone(),
        port: c.port,
        connect_addr: None,
        user: user.into(),
        password: Some(SecretString::from(password)),
        database: db.map(str::to_owned),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
        },
        connect_timeout: Duration::from_secs(10),
    }
}

async fn editor(c: &RedisContainer, db: Option<&str>) -> Box<dyn StoreSession> {
    RedisDriver
        .connect(&target(c, "", &c.password, db), SessionPurpose::Editor)
        .await
        .unwrap()
}

fn argv(parts: &[&str]) -> Vec<Vec<u8>> {
    parts.iter().map(|p| p.as_bytes().to_vec()).collect()
}

async fn run(s: &mut Box<dyn StoreSession>, parts: &[&str]) -> Reply {
    s.run(&argv(parts)).await.unwrap()
}

fn scan_req(cursor: Option<String>, count: u32) -> ScanRequest {
    ScanRequest {
        cursor,
        pattern: None,
        count,
    }
}

async fn all_keys(s: &mut Box<dyn StoreSession>) -> HashSet<Vec<u8>> {
    let (mut keys, mut cursor) = (HashSet::new(), None);
    loop {
        let page = s.scan(&scan_req(cursor, 100)).await.unwrap();
        keys.extend(page.entries.into_iter().map(|e| e.key.0));
        match page.next {
            Some(n) => cursor = Some(n),
            None => return keys,
        }
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn scan_visits_all_10k_keys_in_bounded_pages() {
    // Arrange
    let redis = start_redis(None).await;
    let mut s = editor(&redis, None).await;
    let mut mset = vec!["MSET".to_owned()];
    for i in 0..10_000 {
        mset.extend([format!("k:{i}"), i.to_string()]);
    }
    let mset: Vec<Vec<u8>> = mset.into_iter().map(String::into_bytes).collect();
    s.run(&mset).await.unwrap();
    let count = 500;

    // Act
    let (mut seen, mut cursor, mut largest) = (HashSet::new(), None, 0);
    loop {
        let page = s.scan(&scan_req(cursor, count)).await.unwrap();
        largest = largest.max(page.entries.len());
        seen.extend(page.entries.into_iter().map(|e| e.key.0));
        match page.next {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }

    // Assert: COUNT is only a hint, so allow some slack per page.
    assert_eq!(seen.len(), 10_000);
    assert!(largest <= 2 * count as usize, "page of {largest} keys");
}

#[tokio::test]
#[ignore = "docker"]
async fn reads_each_type_with_ttl_and_size() {
    // Arrange
    let redis = start_redis(None).await;
    let mut s = editor(&redis, None).await;
    run(&mut s, &["SET", "str", "hello", "PX", "60000"]).await;
    run(&mut s, &["HSET", "hash", "f1", "v1", "f2", "v2"]).await;
    run(&mut s, &["RPUSH", "list", "a", "b", "c"]).await;
    run(&mut s, &["SADD", "set", "x", "y"]).await;
    run(&mut s, &["ZADD", "zset", "1.5", "m1", "2", "m2"]).await;
    run(&mut s, &["XADD", "stream", "1-1", "name", "ada"]).await;

    // Act
    let mut views = Vec::new();
    for k in ["str", "hash", "list", "set", "zset", "stream"] {
        views.push(s.read(&EntryKey(k.into()), 100).await.unwrap());
    }

    // Assert
    let ttl = views[0].ttl.expect("string key expires");
    assert!(ttl <= Duration::from_secs(60) && ttl > Duration::from_secs(50));
    assert!(views[1..].iter().all(|v| v.ttl.is_none()));
    assert!(views.iter().all(|v| !v.truncated));
    assert!(views.iter().all(|v| v.size_bytes.is_some_and(|b| b > 0)));
    assert_eq!(views[0].body, ValueBody::Text(b"hello".to_vec()));
    let ValueBody::Map(mut fields) = views[1].body.clone() else {
        panic!("hash body: {:?}", views[1].body)
    };
    fields.sort();
    assert_eq!(
        fields,
        [
            (b"f1".to_vec(), b"v1".to_vec()),
            (b"f2".to_vec(), b"v2".to_vec())
        ]
    );
    assert_eq!(
        views[2].body,
        ValueBody::List(vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()])
    );
    let ValueBody::Set(mut members) = views[3].body.clone() else {
        panic!("set body: {:?}", views[3].body)
    };
    members.sort();
    assert_eq!(members, [b"x".to_vec(), b"y".to_vec()]);
    assert_eq!(
        views[4].body,
        ValueBody::SortedSet(vec![(b"m1".to_vec(), 1.5), (b"m2".to_vec(), 2.0)])
    );
    let ValueBody::Stream(entries) = &views[5].body else {
        panic!("stream body: {:?}", views[5].body)
    };
    assert_eq!(entries[0].id, "1-1");
    assert_eq!(entries[0].fields, [(b"name".to_vec(), b"ada".to_vec())]);
}

#[tokio::test]
#[ignore = "docker"]
async fn large_hash_read_is_truncated_at_limit() {
    // Arrange
    let redis = start_redis(None).await;
    let mut s = editor(&redis, None).await;
    let mut hset = vec!["HSET".to_owned(), "big".to_owned()];
    for i in 0..500 {
        hset.extend([format!("f{i}"), format!("v{i}")]);
    }
    let hset: Vec<Vec<u8>> = hset.into_iter().map(String::into_bytes).collect();
    s.run(&hset).await.unwrap();
    let key = EntryKey(b"big".to_vec());

    // Act
    let capped = s.read(&key, 100).await.unwrap();
    let whole = s.read(&key, 500).await.unwrap();

    // Assert
    assert!(capped.truncated);
    assert!(matches!(&capped.body, ValueBody::Map(m) if m.len() == 100));
    assert!(!whole.truncated);
    assert!(matches!(&whole.body, ValueBody::Map(m) if m.len() == 500));
}

#[tokio::test]
#[ignore = "docker"]
async fn binary_key_round_trips() {
    // Arrange
    let redis = start_redis(None).await;
    let mut s = editor(&redis, None).await;
    let key = b"\xff\x00bin\x80".to_vec();
    s.run(&[b"SET".to_vec(), key.clone(), b"\x01\x02".to_vec()])
        .await
        .unwrap();

    // Act
    let listed = all_keys(&mut s).await;
    let view = s.read(&EntryKey(key.clone()), 10).await.unwrap();

    // Assert
    assert!(listed.contains(&key));
    assert_eq!(view.body, ValueBody::Text(vec![1, 2]));
}

#[tokio::test]
#[ignore = "docker"]
async fn select_db_isolates_keyspace() {
    // Arrange
    let redis = start_redis(None).await;
    let mut db0 = editor(&redis, Some("0")).await;
    let mut db1 = editor(&redis, Some("1")).await;
    run(&mut db1, &["SET", "only-in-1", "v"]).await;

    // Act
    let in0 = all_keys(&mut db0).await;
    let in1 = all_keys(&mut db1).await;
    let namespaces = db0.namespaces().await.unwrap();

    // Assert
    assert!(in0.is_empty());
    assert!(in1.contains(b"only-in-1".as_slice()));
    assert_eq!(namespaces.len(), 16);
    assert_eq!(namespaces[1], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn acl_user_without_config_still_lists_namespaces() {
    // Arrange
    let redis = start_redis(None).await;
    let mut admin = editor(&redis, None).await;
    run(
        &mut admin,
        &[
            "ACL", "SETUSER", "reader", "on", ">pw", "~*", "+@all", "-config",
        ],
    )
    .await;
    let mut reader = RedisDriver
        .connect(
            &target(&redis, "reader", "pw", Some("2")),
            SessionPurpose::Data,
        )
        .await
        .unwrap();

    // Act
    let namespaces = reader.namespaces().await.unwrap();

    // Assert: CONFIG is denied, so the list comes from INFO keyspace and includes the current db.
    assert!(namespaces.contains(&"2".to_owned()));
}

#[tokio::test]
#[ignore = "docker"]
async fn scan_skips_keys_the_acl_user_cannot_inspect() {
    // Arrange
    let redis = start_redis(None).await;
    let mut admin = editor(&redis, None).await;
    run(&mut admin, &["SET", "a:1", "v"]).await;
    run(&mut admin, &["SET", "b:1", "v"]).await;
    run(
        &mut admin,
        &["ACL", "SETUSER", "r", "on", ">pw", "~a:*", "+@all"],
    )
    .await;
    let mut restricted = RedisDriver
        .connect(&target(&redis, "r", "pw", None), SessionPurpose::Data)
        .await
        .unwrap();

    // Act: SCAN lists b:1 but TYPE/PTTL on it are denied; the page must still come back.
    let keys = all_keys(&mut restricted).await;

    // Assert
    assert_eq!(keys, HashSet::from([b"a:1".to_vec()]));
}

#[tokio::test]
#[ignore = "docker"]
async fn wrong_password_maps_to_auth_error() {
    // Arrange
    let redis = start_redis(None).await;

    // Act
    let err = RedisDriver
        .connect(&target(&redis, "", "nope", None), SessionPurpose::Data)
        .await
        .err()
        .expect("must fail");

    // Assert
    assert_eq!(err.kind, ErrorKind::Auth, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn non_editor_session_rejects_run() {
    // Arrange
    let redis = start_redis(None).await;
    let mut s = RedisDriver
        .connect(
            &target(&redis, "", &redis.password, None),
            SessionPurpose::Data,
        )
        .await
        .unwrap();

    // Act
    let err = s.run(&argv(&["GET", "k"])).await.unwrap_err();

    // Assert
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

async fn client_list(observer: &mut Box<dyn StoreSession>) -> String {
    match run(observer, &["CLIENT", "LIST"]).await {
        Reply::Bulk(b) => String::from_utf8_lossy(&b).into_owned(),
        other => panic!("CLIENT LIST: {other:?}"),
    }
}

async fn client_id(s: &mut Box<dyn StoreSession>) -> i64 {
    match run(s, &["CLIENT", "ID"]).await {
        Reply::Int(i) => i,
        other => panic!("CLIENT ID: {other:?}"),
    }
}

/// Waits until the server no longer lists client `id`; fails after `within`.
async fn assert_client_gone(observer: &mut Box<dyn StoreSession>, id: i64, within: Duration) {
    let marker = format!("id={id} ");
    let start = Instant::now();
    while client_list(observer).await.contains(&marker) {
        assert!(start.elapsed() < within, "client {id} still connected");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn dropping_session_closes_server_client() {
    // Arrange
    let redis = start_redis(None).await;
    let mut observer = editor(&redis, None).await;
    let mut idle = editor(&redis, None).await;
    let idle_id = client_id(&mut idle).await;

    // Act
    drop(idle);

    // Assert
    assert_client_gone(&mut observer, idle_id, Duration::from_secs(2)).await;
}

#[tokio::test]
#[ignore = "docker"]
async fn tls_verify_full_uses_host_over_connect_addr() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let redis = start_redis(Some(&certs)).await;
    let mut t = target(&redis, "", &redis.password, None);
    t.host = "localhost".into();
    t.connect_addr = tokio::net::lookup_host((redis.host.as_str(), redis.port))
        .await
        .unwrap()
        .next();
    t.tls = TlsConfig {
        mode: TlsMode::VerifyFull,
        ca_path: Some(certs.dir.path().join("ca.pem")),
    };

    // Act: dials the tunnel address but validates the certificate against `host`.
    let mut ok = RedisDriver.connect(&t, SessionPurpose::Data).await.unwrap();
    let pong = ok.ping().await;
    t.host = "other.test".into();
    let wrong_name = RedisDriver
        .connect(&t, SessionPurpose::Data)
        .await
        .err()
        .expect("certificate is not valid for other.test");

    // Assert
    assert!(pong.is_ok());
    assert_eq!(ok.server_version().product, Product::Redis);
    assert_eq!(wrong_name.kind, ErrorKind::Tls, "{wrong_name}");
}
