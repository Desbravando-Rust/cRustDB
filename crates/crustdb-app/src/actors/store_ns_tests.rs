#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;

use crustdb_core::model::DriverKind;
use crustdb_core::store::{KeyEntry, ScanPage};
use crustdb_testkit::fake_store::FakeStore;

use super::store_tests::store::{connect, harness, scan, send};
use crate::actors::tests::{WS, next};
use crate::runtime::Command;

#[tokio::test]
async fn switch_namespace_reconnects_with_selected_database() {
    // Arrange
    let mut fake = FakeStore::new(DriverKind::Redis);
    fake.pages = vec![ScanPage {
        entries: Vec::<KeyEntry>::new(),
        next: None,
    }];
    let mut h = harness(fake);
    connect(&mut h).await;

    // Act
    send(
        &mut h,
        Command::SwitchNamespace {
            ws: WS,
            ns: "3".into(),
        },
    );
    send(&mut h, scan(1, None));
    next(&h.rx).await;

    // Assert
    assert_eq!(h.log.connects.load(Ordering::SeqCst), 2);
    assert_eq!(
        h.log.databases.lock().unwrap().last(),
        Some(&Some("3".to_string()))
    );
}
