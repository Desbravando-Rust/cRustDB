#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::{Duration, Instant};

use crustdb_core::model::{DriverKind, Product, ServerVersion};
use crustdb_core::safety::Verdict;
use crustdb_core::store::Reply;
use secrecy::SecretString;

use crate::runtime::{Command, Event, RequestId, WorkspaceId};
use crate::state::workspace::tests::{conn, state};
use crate::state::{AppState, Dialog};

fn online_redis() -> (AppState, WorkspaceId) {
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
    s.take_commands();
    (s, ws)
}

fn pane(s: &mut AppState, ws: WorkspaceId) -> &mut super::CommandPane {
    &mut s
        .workspace_mut(ws)
        .unwrap()
        .keyspace
        .as_mut()
        .unwrap()
        .command
}

/// Types `text`, runs it and returns the assess request id.
fn submit(s: &mut AppState, ws: WorkspaceId, text: &str) -> RequestId {
    pane(s, ws).text = text.into();
    let Some(Command::AssessCommand { req, .. }) = s.run_command(ws) else {
        panic!("expected AssessCommand");
    };
    req
}

fn later() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

#[test]
fn flushdb_in_editor_opens_password_dialog() {
    // Arrange
    let (mut s, ws) = online_redis();
    let req = submit(&mut s, ws, "FLUSHDB");

    // Act
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((Verdict::Password, "can wipe data".into())),
    });

    // Assert
    assert!(matches!(
        s.dialogs.top(),
        Some(Dialog::StoreSafety { verdict: Verdict::Password, reason, .. }) if reason == "can wipe data"
    ));
    assert!(s.take_commands().is_empty(), "nothing runs before approval");
}

#[test]
fn password_approval_runs_the_stored_argv_and_wrong_password_does_not() {
    // Arrange
    let (mut s, ws) = online_redis();
    s.workspace_mut(ws).unwrap().session_password = Some(SecretString::from("s3cret"));
    let req = submit(&mut s, ws, "FLUSHDB");
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((Verdict::Password, "wipe".into())),
    });
    let type_pw = |s: &mut AppState, pw: &str| {
        let Some(Dialog::StoreSafety { input, .. }) = s.dialogs.top_mut() else {
            panic!("no dialog");
        };
        *input = crate::state::dialogs::PasswordInput::new(pw);
    };

    // Act
    type_pw(&mut s, "wrong");
    let denied = s.confirm_safety(later());
    type_pw(&mut s, "s3cret");
    let approved = s.confirm_safety(later());

    // Assert
    assert!(denied.is_none());
    assert!(
        matches!(approved, Some(Command::RunCommand { req: r, argv, .. }) if r == req && argv == [b"FLUSHDB".to_vec()])
    );
    assert!(s.dialogs.top().is_none());
    assert!(pane(&mut s, ws).is_running());
}

#[test]
fn run_verdict_goes_straight_to_run_command() {
    // Arrange
    let (mut s, ws) = online_redis();
    let req = submit(&mut s, ws, "GET \"a b\"");

    // Act
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((Verdict::Run, String::new())),
    });

    // Assert
    let cmds = s.take_commands();
    assert!(
        matches!(&cmds[..], [Command::RunCommand { argv, .. }] if argv == &[b"GET".to_vec(), b"a b".to_vec()])
    );
    assert!(s.dialogs.top().is_none());
}

#[test]
fn multi_and_subscribe_refusal_is_visible() {
    // Arrange
    let (mut s, ws) = online_redis();
    let req = submit(&mut s, ws, "SUBSCRIBE ch");

    // Act
    s.apply(Event::CommandAssessed {
        req,
        result: Err("SUBSCRIBE is not supported in this client".into()),
    });

    // Assert
    assert!(s.dialogs.top().is_none());
    assert!(s.take_commands().is_empty());
    let error = pane(&mut s, ws).error.clone().unwrap();
    assert!(error.contains("SUBSCRIBE is not supported"));
}

#[test]
fn unbalanced_quotes_show_an_error_and_send_nothing() {
    // Arrange
    let (mut s, ws) = online_redis();
    pane(&mut s, ws).text = "SET k \"oops".into();

    // Act
    let cmd = s.run_command(ws);

    // Assert
    assert!(cmd.is_none());
    assert!(pane(&mut s, ws).error.is_some());
}

#[test]
fn reply_fills_the_grid_and_cancel_targets_the_running_request() {
    // Arrange
    let (mut s, ws) = online_redis();
    let req = submit(&mut s, ws, "LRANGE l 0 -1");
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((Verdict::Run, String::new())),
    });
    let cancel = s.cancel_command(ws);

    // Act
    s.apply(Event::CommandReply {
        req,
        result: Ok(Reply::Array(vec![Reply::Int(1), Reply::Int(2)])),
    });

    // Assert
    assert!(matches!(cancel, Some(Command::CancelStore { req: r, .. }) if r == req));
    let p = pane(&mut s, ws);
    assert_eq!(p.rows.len(), 2);
    assert!(!p.is_running());
}

#[test]
fn stale_reply_is_ignored() {
    // Arrange
    let (mut s, ws) = online_redis();
    let req = submit(&mut s, ws, "PING");
    s.apply(Event::CommandAssessed {
        req,
        result: Ok((Verdict::Run, String::new())),
    });

    // Act
    s.apply(Event::CommandReply {
        req: RequestId(req.0 + 99),
        result: Ok(Reply::Nil),
    });

    // Assert
    assert!(pane(&mut s, ws).is_running());
}
