#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::safety::{Flag, FlagReason};

use super::*;

const WS: WorkspaceId = WorkspaceId(2);
const TAB: TabId = TabId(7);
const REQ: RequestId = RequestId(10);
const EXEC: RequestId = RequestId(11);

fn eval(verdict: Verdict) -> Evaluation {
    let flags = if verdict == Verdict::Run {
        vec![]
    } else {
        vec![Flag {
            index: 0,
            verdict,
            reason: FlagReason::NoWhere,
        }]
    };
    Evaluation { verdict, flags }
}

fn execute() -> Command {
    Command::Execute {
        tab: TAB,
        req: EXEC,
        evaluation: REQ,
    }
}

fn with_dialog(v: Verdict) -> DialogStack {
    let mut s = DialogStack::default();
    assert!(
        s.on_evaluated(WS, TAB, REQ, EXEC, eval(v), Instant::now())
            .is_none()
    );
    s
}

fn later() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

fn submit(s: &mut DialogStack, pw: Option<&SecretString>, db: &str) -> Option<Command> {
    s.submit_safety(
        |ws| {
            (ws == WS).then_some(SessionAuth {
                password: pw,
                database: db,
            })
        },
        later(),
    )
}

fn type_pw(s: &mut DialogStack, pw: &str) {
    let Some(Dialog::Safety { input, .. }) = s.top_mut() else {
        panic!("no safety dialog");
    };
    *input = PasswordInput::new(pw);
}

#[test]
fn run_verdict_executes_without_dialog() {
    let mut s = DialogStack::default();
    let cmd = s.on_evaluated(WS, TAB, REQ, EXEC, eval(Verdict::Run), Instant::now());
    assert!(matches!(cmd, Some(c) if format!("{c:?}") == format!("{:?}", execute())));
    assert!(s.top().is_none());
}

#[test]
fn confirm_executes_once_with_evaluated_id() {
    let mut s = with_dialog(Verdict::Confirm);
    let cmd = submit(&mut s, None, "db").unwrap();
    assert_eq!(format!("{cmd:?}"), format!("{:?}", execute()));
    assert!(s.top().is_none());
    assert!(submit(&mut s, None, "db").is_none());
}

#[test]
fn confirm_dialog_lists_flagged_statements_with_reason_and_estimate() {
    let mut ev = eval(Verdict::Confirm);
    ev.flags = vec![
        Flag {
            index: 1,
            verdict: Verdict::Confirm,
            reason: FlagReason::MultiRow(42),
        },
        Flag {
            index: 3,
            verdict: Verdict::Confirm,
            reason: FlagReason::NoWhere,
        },
    ];
    let rows = crate::ui::confirm_dialog::flag_rows(&ev);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with("Statement 2:") && rows[0].contains("42"));
    assert!(rows[1].starts_with("Statement 4:") && rows[1].contains("WHERE"));
}

#[test]
fn password_flow_three_failures_aborts() {
    let mut s = with_dialog(Verdict::Password);
    let pw = SecretString::from("hunter2");
    for n in 1..=3 {
        type_pw(&mut s, "wrong");
        assert!(submit(&mut s, Some(&pw), "db").is_none());
        if n < 3 {
            let Some(Dialog::Safety {
                attempts, input, ..
            }) = s.top()
            else {
                panic!("closed early");
            };
            assert_eq!(*attempts, n);
            assert!(input.0.is_empty());
        }
    }
    assert!(s.top().is_none());
    assert!(submit(&mut s, Some(&pw), "db").is_none());
}

#[test]
fn password_right_after_failure_executes() {
    let mut s = with_dialog(Verdict::Password);
    let pw = SecretString::from("hunter2");
    type_pw(&mut s, "nope");
    assert!(submit(&mut s, Some(&pw), "db").is_none());
    type_pw(&mut s, "hunter2");
    let cmd = submit(&mut s, Some(&pw), "db").unwrap();
    assert_eq!(format!("{cmd:?}"), format!("{:?}", execute()));
}

#[test]
fn password_without_db_password_requires_database_name() {
    assert!(verify_password("shop", None, "shop"));
    assert!(!verify_password("hunter2", None, "shop"));
    assert!(!verify_password("", None, ""));
    let mut s = with_dialog(Verdict::Password);
    type_pw(&mut s, "shop");
    assert!(submit(&mut s, None, "shop").is_some());
}

#[test]
fn constant_time_compare_rejects_prefix() {
    let pw = SecretString::from("hunter2");
    assert!(!verify_password("hunter", Some(&pw), "db"));
    assert!(!verify_password("hunter22", Some(&pw), "db"));
    assert!(!verify_password("", Some(&pw), "db"));
    assert!(verify_password("hunter2", Some(&pw), "db"));
}

#[test]
fn cancel_sends_nothing() {
    let mut s = with_dialog(Verdict::Password);
    assert!(s.cancel().is_none());
    assert!(s.top().is_none());
    assert!(submit(&mut s, None, "db").is_none());
}

#[test]
fn host_key_dialog_sends_decision() {
    let hk = || Dialog::HostKey {
        ws: WorkspaceId(3),
        host: "h".into(),
        fingerprint: "SHA256:x".into(),
    };
    let mut s = DialogStack::default();
    s.push(hk());
    assert!(matches!(
        s.accept_host_key(),
        Some(Command::HostKeyDecision {
            ws: WorkspaceId(3),
            accept: true
        })
    ));
    s.push(hk());
    assert!(matches!(
        s.cancel(),
        Some(Command::HostKeyDecision { accept: false, .. })
    ));
    assert!(s.accept_host_key().is_none());
}

#[test]
fn close_with_tx_resolves_or_cancels() {
    let mut s = DialogStack::default();
    s.push(Dialog::CloseWithTx { tab: TAB });
    assert!(s.close_with_tx(TxOp::Savepoint).is_none());
    assert!(s.top().is_some());
    assert!(matches!(
        s.close_with_tx(TxOp::Rollback),
        Some(Command::CloseTab {
            resolve: Some(TxOp::Rollback),
            ..
        })
    ));
    s.push(Dialog::CloseWithTx { tab: TAB });
    assert!(s.cancel().is_none());
}

#[test]
fn dirty_file_discard_closes_tab() {
    let mut s = DialogStack::default();
    s.push(Dialog::DirtyFile { tab: TAB });
    assert!(matches!(
        s.discard_dirty(),
        Some(Command::CloseTab { resolve: None, .. })
    ));
    assert!(s.top().is_none());
}

#[test]
fn different_lengths_never_match_even_with_zero_padding() {
    let pw = SecretString::from("ab");
    assert!(!verify_password("ab\0", Some(&pw), "db"));
    assert!(!verify_password(
        "ab",
        Some(&SecretString::from("ab\0")),
        "db"
    ));
    let long = "x".repeat(300);
    assert!(verify_password(
        &long,
        Some(&SecretString::from(long.clone())),
        "db"
    ));
    assert!(!verify_password(
        &long,
        Some(&SecretString::from(long[..299].to_owned())),
        "db"
    ));
}

#[test]
fn empty_session_password_falls_back_to_database_name() {
    let empty = SecretString::from("");
    assert!(verify_password("shop", Some(&empty), "shop"));
    assert!(!verify_password("", Some(&empty), ""));
}

#[test]
fn credentials_come_from_the_dialogs_workspace() {
    let mut s = with_dialog(Verdict::Password);
    type_pw(&mut s, "dev");
    // Another workspace with database "dev" is irrelevant; WS has the real password.
    let pw = SecretString::from("prod-secret");
    assert!(submit(&mut s, Some(&pw), "dev").is_none());
    // Workspace gone: dialog dropped, nothing executes.
    let mut s = with_dialog(Verdict::Confirm);
    assert!(s.submit_safety(|_| None, later()).is_none());
    assert!(s.top().is_none());
}

#[test]
fn flag_level_password_requires_password_even_if_batch_says_confirm() {
    let mut ev = eval(Verdict::Confirm);
    ev.flags[0].verdict = Verdict::Password;
    let mut s = DialogStack::default();
    s.on_evaluated(WS, TAB, REQ, EXEC, ev, Instant::now());
    assert!(submit(&mut s, None, "db").is_none());
    assert!(s.top().is_some());
}

#[test]
fn approval_inside_grace_window_is_ignored() {
    let mut s = with_dialog(Verdict::Confirm);
    let now = Instant::now();
    let early = s.submit_safety(
        |_| {
            Some(SessionAuth {
                password: None,
                database: "db",
            })
        },
        now,
    );
    assert!(early.is_none());
    assert!(s.top().is_some());
    assert!(submit(&mut s, None, "db").is_some());
}

#[test]
fn debug_never_prints_the_typed_password() {
    let mut s = with_dialog(Verdict::Password);
    type_pw(&mut s, "topsecret");
    assert!(!format!("{s:?}").contains("topsecret"));
}
