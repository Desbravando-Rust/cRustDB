#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::DriverKind;
use crustdb_testkit::fake_driver::FakeScript;

use super::tests::{WS, config, harness, next};
use crate::runtime::{Command, Event, ResolvedSecrets};

#[tokio::test]
async fn redis_connection_without_store_driver_fails_cleanly() {
    // Arrange
    let mut h = harness(FakeScript::default());
    let mut cfg = config(None);
    cfg.driver = DriverKind::Redis;

    // Act
    let left = h.actors.dispatch(Command::Connect {
        ws: WS,
        config: cfg,
        secrets: ResolvedSecrets::default(),
    });

    // Assert
    assert!(left.is_none());
    assert!(matches!(
        next(&h.rx).await,
        Event::ConnectFailed { error, .. } if error == "no driver for this connection"
    ));
}

pub(in crate::actors) mod store {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use crustdb_core::driver::SessionPurpose;
    use crustdb_core::model::{DriverKind, ObjectKind};
    use crustdb_core::safety::Verdict;
    use crustdb_core::store::keyspace::EntryKey;
    use crustdb_core::store::value::ValueBody;
    use crustdb_core::store::{EntryView, KeyEntry, Reply, ScanPage, ValueKind};
    use crustdb_testkit::fake_store::{FakeStore, FakeStoreLog};

    use crate::actors::Actors;
    use crate::actors::tests::{WS, config, drain, next};
    use crate::runtime::{
        Command, DriverRegistry, Emitter, Event, ROWS_BATCH, RequestId, ResolvedSecrets,
    };

    pub(in crate::actors) struct H {
        pub(in crate::actors) actors: Actors,
        pub(in crate::actors) rx: mpsc::Receiver<Event>,
        pub(in crate::actors) log: Arc<FakeStoreLog>,
    }

    fn entry(key: &str) -> KeyEntry {
        KeyEntry {
            key: EntryKey(key.as_bytes().to_vec()),
            kind: ValueKind::Text,
            ttl: None,
        }
    }

    fn page(keys: &[&str], next: Option<&str>) -> ScanPage {
        ScanPage {
            entries: keys.iter().map(|k| entry(k)).collect(),
            next: next.map(String::from),
        }
    }

    fn view() -> EntryView {
        EntryView {
            ttl: None,
            size_bytes: Some(1),
            body: ValueBody::Text(b"v".to_vec()),
            truncated: false,
        }
    }

    pub(in crate::actors) fn harness(fake: FakeStore) -> H {
        let log = fake.log.clone();
        let (tx, rx) = mpsc::channel();
        let out = Emitter {
            tx,
            repaint: Arc::new(|| {}),
        };
        let reg = DriverRegistry::default().with_store(Arc::new(fake));
        H {
            actors: Actors::new(reg, out),
            rx,
            log,
        }
    }

    pub(in crate::actors) fn send(h: &mut H, cmd: Command) {
        assert!(h.actors.dispatch(cmd).is_none());
    }

    pub(in crate::actors) async fn connect(h: &mut H) -> Event {
        let mut cfg = config(None);
        cfg.driver = DriverKind::Redis;
        send(
            h,
            Command::Connect {
                ws: WS,
                config: cfg,
                secrets: ResolvedSecrets::default(),
            },
        );
        next(&h.rx).await
    }

    pub(in crate::actors) fn scan(req: u64, cursor: Option<&str>) -> Command {
        Command::ScanKeys {
            ws: WS,
            req: RequestId(req),
            cursor: cursor.map(String::from),
            pattern: None,
        }
    }

    #[tokio::test]
    async fn store_connect_emits_connected_with_server_version() {
        // Arrange
        let mut h = harness(FakeStore::new(DriverKind::Redis));

        // Act
        let ev = connect(&mut h).await;

        // Assert
        assert!(matches!(ev, Event::Connected { ws: WS, server } if server.raw == "fake-store"));
        assert_eq!(h.log.connects.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keyspace_loads_lazily_in_scan_batches() {
        // Arrange
        let mut fake = FakeStore::new(DriverKind::Redis);
        fake.pages = vec![page(&["a"], Some("1")), page(&["b"], None)];
        let mut h = harness(fake);
        connect(&mut h).await;

        // Act
        send(&mut h, scan(1, None));
        let first = next(&h.rx).await;
        send(
            &mut h,
            Command::ListNamespaces {
                ws: WS,
                req: RequestId(9),
            },
        );
        next(&h.rx).await;

        // Assert
        let Event::KeysScanned { req, result } = first else {
            panic!("expected KeysScanned, got {first:?}");
        };
        assert_eq!(req, RequestId(1));
        assert_eq!(result.unwrap().next.as_deref(), Some("1"));
        assert_eq!(
            h.log.ops(),
            [format!("scan:-:{ROWS_BATCH}"), "namespaces".to_string()],
            "the mailbox was served in order with no scan continuation"
        );
        assert!(drain(&h.rx).is_empty(), "no automatic continuation");

        // Act
        send(&mut h, scan(2, Some("1")));
        let second = next(&h.rx).await;

        // Assert
        assert!(matches!(second, Event::KeysScanned { result: Ok(p), .. } if p.next.is_none()));
        assert_eq!(h.log.ops().len(), 3);
    }

    #[tokio::test]
    async fn namespaces_and_read_answer_from_store_session() {
        // Arrange
        let mut fake = FakeStore::new(DriverKind::Redis);
        fake.namespaces = vec!["0".into(), "1".into()];
        fake.entries = vec![(EntryKey(b"k".to_vec()), view())];
        let mut h = harness(fake);
        connect(&mut h).await;

        // Act
        send(
            &mut h,
            Command::ListNamespaces {
                ws: WS,
                req: RequestId(1),
            },
        );
        let namespaces = next(&h.rx).await;
        send(
            &mut h,
            Command::ReadEntry {
                ws: WS,
                req: RequestId(2),
                key: EntryKey(b"k".to_vec()),
            },
        );
        let read = next(&h.rx).await;
        send(
            &mut h,
            Command::ReadEntry {
                ws: WS,
                req: RequestId(3),
                key: EntryKey(b"x".to_vec()),
            },
        );
        let missing = next(&h.rx).await;

        // Assert
        assert!(matches!(namespaces, Event::Namespaces { result: Ok(n), .. } if n == ["0", "1"]));
        assert!(matches!(read, Event::EntryRead { result: Ok(v), .. } if v == view()));
        assert!(matches!(missing, Event::EntryRead { result: Err(e), .. } if e == "no such key"));
        assert_eq!(
            h.log.ops()[1],
            "read:k:300",
            "read is limited by the page size"
        );
    }

    #[tokio::test]
    async fn cancel_drops_store_session_and_reconnects_on_demand() {
        // Arrange
        let mut fake = FakeStore::new(DriverKind::Redis);
        fake.pages = vec![page(&["a"], None)];
        let mut h = harness(fake);
        connect(&mut h).await;
        h.log.hang_scan.store(true, Ordering::SeqCst);
        send(&mut h, scan(1, None));
        while h.log.ops().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // Act
        send(
            &mut h,
            Command::CancelStore {
                ws: WS,
                req: RequestId(1),
            },
        );
        let cancelled = next(&h.rx).await;

        // Assert
        assert!(matches!(cancelled, Event::KeysScanned { result: Err(e), .. } if e == "cancelled"));
        assert_eq!(
            h.log.alive.load(Ordering::SeqCst),
            0,
            "the session was dropped"
        );

        // Act
        h.log.hang_scan.store(false, Ordering::SeqCst);
        send(&mut h, scan(2, None));
        let again = next(&h.rx).await;

        // Assert
        assert!(matches!(again, Event::KeysScanned { result: Ok(_), .. }));
        assert_eq!(h.log.connects.load(Ordering::SeqCst), 2);
    }

    fn read(req: u64) -> Command {
        Command::ReadEntry {
            ws: WS,
            req: RequestId(req),
            key: EntryKey(b"k".to_vec()),
        }
    }

    fn cancel(req: u64) -> Command {
        Command::CancelStore {
            ws: WS,
            req: RequestId(req),
        }
    }

    async fn hung_scan_then_read(h: &mut H) {
        connect(h).await;
        h.log.hang_scan.store(true, Ordering::SeqCst);
        send(h, scan(1, None));
        while h.log.ops().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        send(h, read(2));
    }

    fn store_with_entry() -> FakeStore {
        let mut fake = FakeStore::new(DriverKind::Redis);
        fake.entries = vec![(EntryKey(b"k".to_vec()), view())];
        fake
    }

    #[tokio::test]
    async fn cancel_aborts_only_the_named_request_in_flight() {
        // Arrange
        let mut h = harness(store_with_entry());
        hung_scan_then_read(&mut h).await;

        // Act
        send(&mut h, cancel(1));
        let scanned = next(&h.rx).await;
        let read = next(&h.rx).await;

        // Assert
        assert!(
            matches!(scanned, Event::KeysScanned { req: RequestId(1), result: Err(e) } if e == "cancelled")
        );
        assert!(matches!(
            read,
            Event::EntryRead {
                req: RequestId(2),
                result: Ok(_)
            }
        ));
    }

    #[tokio::test]
    async fn cancel_discards_a_queued_request_without_touching_the_one_in_flight() {
        // Arrange
        let mut h = harness(store_with_entry());
        hung_scan_then_read(&mut h).await;

        // Act
        send(&mut h, cancel(2));
        let read = next(&h.rx).await;
        send(&mut h, cancel(1));
        let scanned = next(&h.rx).await;

        // Assert
        assert!(
            matches!(read, Event::EntryRead { req: RequestId(2), result: Err(e) } if e == "cancelled")
        );
        assert!(
            matches!(scanned, Event::KeysScanned { req: RequestId(1), result: Err(e) } if e == "cancelled")
        );
    }

    #[tokio::test]
    async fn late_cancel_of_a_finished_request_keeps_the_next_one_and_the_session() {
        // Arrange
        let mut fake = FakeStore::new(DriverKind::Redis);
        fake.pages = vec![page(&["a"], None)];
        let mut h = harness(fake);
        connect(&mut h).await;
        send(&mut h, scan(1, None));
        next(&h.rx).await;
        h.log.hang_scan.store(true, Ordering::SeqCst);
        send(&mut h, scan(2, None));
        while h.log.ops().len() < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // Act
        send(&mut h, cancel(1));
        send(&mut h, cancel(2));
        let first = next(&h.rx).await;

        // Assert
        assert!(
            matches!(first, Event::KeysScanned { req: RequestId(2), result: Err(e) } if e == "cancelled"),
            "cancel(1) emitted nothing"
        );
        assert_eq!(h.log.connects.load(Ordering::SeqCst), 1, "session kept");
    }

    #[tokio::test]
    async fn sql_commands_on_store_workspace_get_empty_answer() {
        // Arrange
        let mut h = harness(FakeStore::new(DriverKind::Redis));
        connect(&mut h).await;

        // Act
        send(
            &mut h,
            Command::LoadChildren {
                ws: WS,
                req: RequestId(7),
                database: "0".into(),
                schema: None,
                kind: Some(ObjectKind::Table),
            },
        );
        let first = next(&h.rx).await;
        let second = next(&h.rx).await;

        // Assert
        assert!(matches!(first, Event::Notice { .. }));
        assert!(matches!(second, Event::Children { req: RequestId(7), items } if items.is_empty()));
    }

    fn assess(req: u64, argv: &[Vec<u8>]) -> Command {
        Command::AssessCommand {
            ws: WS,
            req: RequestId(req),
            argv: argv.to_vec(),
        }
    }

    fn run_cmd(req: u64, argv: &[Vec<u8>]) -> Command {
        Command::RunCommand {
            ws: WS,
            req: RequestId(req),
            argv: argv.to_vec(),
        }
    }

    #[tokio::test]
    async fn quoted_crlf_command_runs_as_single_argument() {
        // Arrange
        let mut h = harness(FakeStore::new(DriverKind::Redis));
        connect(&mut h).await;
        let argv = crustdb_redis::tokenize::tokenize("SET k \"v\r\nFLUSHALL\"").unwrap();

        // Act
        send(&mut h, assess(1, &argv));
        let assessed = next(&h.rx).await;
        send(&mut h, run_cmd(1, &argv));
        let reply = next(&h.rx).await;

        // Assert
        assert!(matches!(
            assessed,
            Event::CommandAssessed {
                result: Ok((Verdict::Run, _)),
                ..
            }
        ));
        assert!(matches!(
            reply,
            Event::CommandReply {
                result: Ok(Reply::Int(3)),
                ..
            }
        ));
        assert_eq!(h.log.ops(), ["run:3"], "CRLF stayed inside one argument");
        assert!(h.log.purposes().contains(&SessionPurpose::Editor));
    }

    #[tokio::test]
    async fn cancel_drops_editor_session_and_reconnects_on_demand() {
        // Arrange
        let mut h = harness(FakeStore::new(DriverKind::Redis));
        connect(&mut h).await;
        let argv = vec![b"PING".to_vec()];
        h.log.hang_run.store(true, Ordering::SeqCst);
        send(&mut h, run_cmd(1, &argv));
        while h.log.ops().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // Act
        send(&mut h, cancel(1));
        let cancelled = next(&h.rx).await;

        // Assert
        assert!(
            matches!(cancelled, Event::CommandReply { result: Err(e), .. } if e == "cancelled")
        );
        assert_eq!(
            h.log.alive.load(Ordering::SeqCst),
            1,
            "only the data session survives"
        );

        // Act
        h.log.hang_run.store(false, Ordering::SeqCst);
        send(&mut h, run_cmd(2, &argv));
        let again = next(&h.rx).await;

        // Assert
        assert!(matches!(again, Event::CommandReply { result: Ok(_), .. }));
        let editors = h
            .log
            .purposes()
            .iter()
            .filter(|p| **p == SessionPurpose::Editor)
            .count();
        assert_eq!(editors, 2);
    }
}
