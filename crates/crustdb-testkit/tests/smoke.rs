#![allow(clippy::unwrap_used)]

use crustdb_testkit::{certs_for, start_mysql, start_postgres, start_sshd};
use tokio::net::TcpStream;

#[tokio::test]
#[ignore = "docker"]
async fn containers_start_and_accept_connections() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres(Some(&certs), None).await;
    let my = start_mysql(Some(&certs), None).await;
    let ssh = start_sshd("crustdb-smoke-net").await;

    // Act & Assert
    for (host, port) in [
        (&pg.host, pg.port),
        (&my.host, my.port),
        (&ssh.host, ssh.port),
    ] {
        TcpStream::connect((host.as_str(), port)).await.unwrap();
    }
}
