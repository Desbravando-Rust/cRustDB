#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::Duration;

use crustdb_core::model::{DriverKind, Product, ServerVersion};
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::value::ValueBody;
use crustdb_core::store::{EntryView, KeyEntry, ScanPage, ValueKind};

use crate::runtime::{Command, Event, RequestId, WorkspaceId};
use crate::state::AppState;
use crate::state::workspace::tests::{conn, state};

fn online_redis() -> (AppState, WorkspaceId, Vec<Command>) {
    let mut s = state(&[]);
    let mut c = conn("r");
    c.driver = DriverKind::Redis;
    c.database = Some("0".into());
    let ws = s.open_workspace(c);
    s.apply(Event::Connected {
        ws,
        server: ServerVersion {
            product: Product::Redis,
            major: 7,
            minor: 0,
            patch: 0,
            raw: "7.0.0".into(),
        },
    });
    let cmds = s.take_commands();
    (s, ws, cmds)
}

fn key(k: &str) -> EntryKey {
    EntryKey(k.as_bytes().to_vec())
}

fn page(keys: &[&str], next: Option<&str>) -> ScanPage {
    ScanPage {
        entries: keys
            .iter()
            .map(|k| KeyEntry {
                key: key(k),
                kind: ValueKind::Text,
                ttl: None,
            })
            .collect(),
        next: next.map(String::from),
    }
}

fn scans(cmds: &[Command]) -> Vec<(RequestId, Option<String>, Option<String>)> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::ScanKeys {
                req,
                cursor,
                pattern,
                ..
            } => Some((*req, cursor.clone(), pattern.clone())),
            _ => None,
        })
        .collect()
}

fn labels(s: &mut AppState, ws: WorkspaceId) -> Vec<String> {
    let ks = s.workspace_mut(ws).unwrap().keyspace.as_ref().unwrap();
    ks.nodes().iter().map(|n| n.label.clone()).collect()
}

fn keyspace(s: &mut AppState, ws: WorkspaceId) -> &mut super::Keyspace {
    s.workspace_mut(ws).unwrap().keyspace.as_mut().unwrap()
}

#[test]
fn keyspace_loads_lazily_in_scan_batches() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let first = scans(&cmds);
    let (req, ..) = first[0].clone();

    // Act
    s.apply(Event::KeysScanned {
        req,
        result: Ok(page(&["user:1", "user:2", "cfg"], Some("9"))),
    });
    let auto = s.take_commands();
    let collapsed = labels(&mut s, ws);
    keyspace(&mut s, ws).toggle(b"user");
    let expanded = labels(&mut s, ws);
    let more = keyspace(&mut s, ws).load_more(RequestId(500));

    // Assert
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::ListNamespaces { .. }))
    );
    assert_eq!(first.len(), 1, "one ScanKeys on connect");
    assert_eq!(first[0].1, None);
    assert!(auto.is_empty(), "next batch only on request");
    assert_eq!(collapsed, ["cfg", "user"]);
    assert_eq!(expanded, ["cfg", "user", "1", "2"]);
    assert!(matches!(
        more,
        Some(Command::ScanKeys { cursor: Some(c), .. }) if c == "9"
    ));
}

#[test]
fn scan_error_is_shown_and_retryable() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let (req, ..) = scans(&cmds)[0].clone();
    s.apply(Event::KeysScanned {
        req,
        result: Ok(page(&["a"], Some("7"))),
    });
    let more = keyspace(&mut s, ws).load_more(RequestId(10));
    assert!(more.is_some());

    // Act
    s.apply(Event::KeysScanned {
        req: RequestId(10),
        result: Err("boom".into()),
    });
    let error = keyspace(&mut s, ws).scan_error.clone();
    let retry = keyspace(&mut s, ws).load_more(RequestId(11));

    // Assert
    assert_eq!(error.as_deref(), Some("boom"));
    assert!(matches!(
        retry,
        Some(Command::ScanKeys { cursor: Some(c), .. }) if c == "7"
    ));
    assert_eq!(
        keyspace(&mut s, ws).scan_error,
        None,
        "retry clears the error"
    );
}

#[test]
fn namespaces_and_entry_errors_are_shown() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let ns_req = cmds
        .iter()
        .find_map(|c| match c {
            Command::ListNamespaces { req, .. } => Some(*req),
            _ => None,
        })
        .unwrap();

    // Act
    s.apply(Event::Namespaces {
        req: ns_req,
        result: Err("no acl".into()),
    });
    let ns_error = keyspace(&mut s, ws).ns_error.clone();
    keyspace(&mut s, ws).open(key("a"), RequestId(20));
    s.apply(Event::EntryRead {
        req: RequestId(20),
        result: Err("gone".into()),
    });

    // Assert
    assert_eq!(ns_error.as_deref(), Some("no acl"));
    assert_eq!(keyspace(&mut s, ws).entry_error.as_deref(), Some("gone"));
    assert!(keyspace(&mut s, ws).entry.is_none());
}

#[test]
fn filter_restarts_scan_with_pattern() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let (old, ..) = scans(&cmds)[0].clone();
    s.apply(Event::KeysScanned {
        req: old,
        result: Ok(page(&["a:1"], Some("5"))),
    });
    let late = keyspace(&mut s, ws).load_more(RequestId(30));
    assert!(late.is_some());

    // Act
    let cmd = keyspace(&mut s, ws).set_filter("user:*".into(), RequestId(31));
    s.apply(Event::KeysScanned {
        req: RequestId(30),
        result: Ok(page(&["stale"], None)),
    });
    let after_stale = labels(&mut s, ws);
    s.apply(Event::KeysScanned {
        req: RequestId(31),
        result: Ok(page(&["user:9"], None)),
    });

    // Assert
    assert!(matches!(
        cmd,
        Some(Command::ScanKeys { req: RequestId(31), cursor: None, pattern: Some(p), .. })
            if p == "user:*"
    ));
    assert!(after_stale.is_empty(), "stale answer dropped");
    assert_eq!(labels(&mut s, ws), ["user"]);
}

#[test]
fn opening_key_reads_entry_into_result_grid() {
    // Arrange
    let (mut s, ws, _) = online_redis();

    // Act
    let cmd = keyspace(&mut s, ws).open(key("h"), RequestId(40));
    s.apply(Event::EntryRead {
        req: RequestId(40),
        result: Ok(EntryView {
            ttl: Some(Duration::from_secs(60)),
            size_bytes: Some(12),
            body: ValueBody::Map(vec![(b"f".to_vec(), b"v".to_vec())]),
            truncated: true,
        }),
    });

    // Assert
    assert!(matches!(
        cmd,
        Command::ReadEntry { req: RequestId(40), key: k, .. } if k == key("h")
    ));
    let e = keyspace(&mut s, ws).entry.as_ref().unwrap();
    assert_eq!(e.ttl, Some(Duration::from_secs(60)));
    assert_eq!(e.size_bytes, Some(12));
    assert!(e.truncated);
    assert_eq!(e.columns.len(), 2);
    assert_eq!(e.rows.len(), 1);
}

#[test]
fn ns_and_entry_errors_survive_scan_batches_and_load_more() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let (req, ..) = scans(&cmds)[0].clone();
    let ns_req = cmds
        .iter()
        .find_map(|c| match c {
            Command::ListNamespaces { req, .. } => Some(*req),
            _ => None,
        })
        .unwrap();
    s.apply(Event::Namespaces {
        req: ns_req,
        result: Err("no acl".into()),
    });
    keyspace(&mut s, ws).open(key("a"), RequestId(20));
    s.apply(Event::EntryRead {
        req: RequestId(20),
        result: Err("gone".into()),
    });

    // Act
    s.apply(Event::KeysScanned {
        req,
        result: Ok(page(&["a"], Some("7"))),
    });
    keyspace(&mut s, ws).load_more(RequestId(30));

    // Assert
    let ks = keyspace(&mut s, ws);
    assert_eq!(ks.ns_error.as_deref(), Some("no acl"));
    assert_eq!(ks.entry_error.as_deref(), Some("gone"));
}

#[test]
fn open_keeps_pending_scan_error_for_retry() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let (req, ..) = scans(&cmds)[0].clone();
    s.apply(Event::KeysScanned {
        req,
        result: Err("boom".into()),
    });

    // Act
    keyspace(&mut s, ws).open(key("a"), RequestId(20));

    // Assert
    assert_eq!(keyspace(&mut s, ws).scan_error.as_deref(), Some("boom"));
}

#[test]
fn filter_pattern_is_trimmed() {
    // Arrange
    let (mut s, ws, _) = online_redis();

    // Act
    let cmd = keyspace(&mut s, ws).set_filter("  user:*  ".into(), RequestId(40));

    // Assert
    assert!(matches!(
        cmd,
        Some(Command::ScanKeys { pattern: Some(p), .. }) if p == "user:*"
    ));
}

#[test]
fn select_namespace_resets_tree_and_rescans() {
    // Arrange
    let (mut s, ws, cmds) = online_redis();
    let (req, ..) = scans(&cmds)[0].clone();
    s.apply(Event::KeysScanned {
        req,
        result: Ok(page(&["a:1"], Some("5"))),
    });
    keyspace(&mut s, ws).open(key("a:1"), RequestId(50));
    assert_eq!(keyspace(&mut s, ws).current, "0");

    // Act
    let cmds = s.select_namespace(ws, "3");

    // Assert
    assert!(matches!(
        &cmds[..],
        [
            Command::SwitchNamespace { ns, .. },
            Command::ScanKeys { cursor: None, .. }
        ] if ns == "3"
    ));
    let ks = keyspace(&mut s, ws);
    assert_eq!(ks.current, "3");
    assert!(ks.entry.is_none() && ks.nodes().is_empty() && ks.scan.entries.is_empty());
}

#[test]
fn select_current_namespace_is_noop() {
    // Arrange
    let (mut s, ws, _) = online_redis();

    // Act
    let cmds = s.select_namespace(ws, "0");

    // Assert
    assert!(cmds.is_empty());
}

#[test]
fn select_namespace_refused_while_command_running() {
    // Arrange
    let (mut s, ws, _) = online_redis();
    keyspace(&mut s, ws).command.text = "GET k".into();
    let Some(Command::AssessCommand { req, .. }) = s.run_command(ws) else {
        panic!("expected AssessCommand");
    };
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((crustdb_core::safety::Verdict::Run, String::new())),
    });
    assert!(keyspace(&mut s, ws).command.is_running());

    // Act
    let cmds = s.select_namespace(ws, "3");

    // Assert
    assert!(cmds.is_empty());
    assert_eq!(keyspace(&mut s, ws).current, "0");
}

#[test]
fn select_namespace_refused_while_command_is_assessing() {
    // Arrange
    let (mut s, ws, _) = online_redis();
    keyspace(&mut s, ws).command.text = "GET k".into();
    assert!(s.run_command(ws).is_some());

    // Act
    let cmds = s.select_namespace(ws, "3");

    // Assert
    assert!(cmds.is_empty());
    assert_eq!(keyspace(&mut s, ws).current, "0");
}

#[test]
fn select_namespace_refused_while_safety_dialog_open() {
    // Arrange
    let (mut s, ws, _) = online_redis();
    keyspace(&mut s, ws).command.text = "FLUSHDB".into();
    let Some(Command::AssessCommand { req, .. }) = s.run_command(ws) else {
        panic!("expected AssessCommand");
    };
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((
            crustdb_core::safety::Verdict::Password,
            "can wipe data".into(),
        )),
    });
    assert!(s.dialogs.has_store_safety(ws));

    // Act
    let cmds = s.select_namespace(ws, "3");

    // Assert
    assert!(cmds.is_empty());
    assert_eq!(keyspace(&mut s, ws).current, "0");
}

#[test]
fn select_namespace_keeps_the_filter_pattern() {
    // Arrange
    let (mut s, ws, _) = online_redis();
    keyspace(&mut s, ws).set_filter("user:*".into(), RequestId(60));

    // Act
    let cmds = s.select_namespace(ws, "3");

    // Assert
    assert!(matches!(
        &cmds[..],
        [_, Command::ScanKeys { pattern: Some(p), .. }] if p == "user:*"
    ));
}
