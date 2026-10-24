#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_testkit::fake_driver::FakeScript;

use super::editor::tests::open;
use super::tests::{TAB, WS, harness};
use crate::runtime::{Command, RequestId};

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
