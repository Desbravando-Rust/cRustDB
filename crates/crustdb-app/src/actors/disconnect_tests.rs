#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_testkit::fake_driver::FakeScript;

use super::editor::tests::open;
use super::editor::tests::{settle, tx_states};
use super::tests::{TAB, WS, harness};
use crate::runtime::{Command, RequestId, TxOp, TxState};

#[tokio::test(start_paused = true)]
async fn disconnect_drops_export_sender_of_the_editor_tab() {
    // Arrange
    let mut h = harness(FakeScript::default());
    open(&mut h).await;
    let path = std::path::PathBuf::from("/tmp/out.csv");
    h.actors.dispatch(Command::ExportResult {
        tab: TAB,
        req: RequestId(1),
        path,
    });
    assert_eq!(h.actors.exports.len(), 1);

    // Act
    h.actors.dispatch(Command::Disconnect { ws: WS });

    // Assert
    assert!(h.actors.exports.is_empty());
}

#[tokio::test(start_paused = true)]
async fn tx_commit_resolves_open_tx_and_keeps_editor_alive() {
    // Arrange
    let mut h = harness(FakeScript::default());
    open(&mut h).await;
    h.actors.dispatch(Command::Tx {
        tab: TAB,
        op: TxOp::Begin,
    });
    settle(&h).await;

    // Act
    h.actors.dispatch(Command::Tx {
        tab: TAB,
        op: TxOp::Commit,
    });
    let ev = settle(&h).await;

    // Assert
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    assert!(
        h.drv
            .log
            .ops
            .lock()
            .unwrap()
            .iter()
            .any(|q| q.starts_with("start:") && q.contains("COMMIT"))
    );
}
