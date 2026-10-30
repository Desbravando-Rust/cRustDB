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
