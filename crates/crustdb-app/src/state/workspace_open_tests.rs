#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::tests::state;
use super::*;

#[test]
fn open_workspace_replaces_failed_workspace_of_same_connection() {
    // Arrange
    let mut s = state(&["a", "b"]);
    let (a, b) = (
        s.config.connections[0].clone(),
        s.config.connections[1].clone(),
    );
    let failed_a = s.open_workspace(a.clone());
    s.apply(Event::ConnectFailed {
        ws: failed_a,
        error: "no password".into(),
    });
    let failed_b = s.open_workspace(b);
    s.apply(Event::ConnectFailed {
        ws: failed_b,
        error: "refused".into(),
    });

    // Act
    let new_a = s.open_workspace(a.clone());

    // Assert
    let of_a: Vec<_> = s
        .workspaces
        .iter()
        .filter(|w| w.config.id == a.id)
        .collect();
    assert_eq!(of_a.len(), 1);
    assert_eq!(of_a[0].id, new_a);
    assert_eq!(of_a[0].status, Status::Connecting);
    assert!(s.workspaces.iter().any(|w| w.id == failed_b));
    assert_eq!(s.active, Some(new_a));
}
