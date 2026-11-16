use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn accept_connection(listener: &TcpListener) -> Option<(TcpStream, SocketAddr)> {
    match timeout(ACCEPT_POLL_INTERVAL, listener.accept()).await {
        Ok(Ok(connection)) => Some(connection),
        Ok(Err(error)) => {
            tracing::error!(error = ?error, "HTTP accept failed");
            sleep(ACCEPT_POLL_INTERVAL).await;
            None
        }
        Err(_) => None,
    }
}

pub(super) async fn drain_connections(connections: &mut JoinSet<()>) {
    if timeout(CONNECTION_DRAIN_TIMEOUT, async {
        while let Some(result) = connections.join_next().await {
            if let Err(error) = result {
                tracing::error!(error = ?error, "HTTP connection task stopped during shutdown");
            }
        }
    })
    .await
    .is_err()
    {
        tracing::warn!("HTTP connection drain timed out");
        connections.shutdown().await;
    }
}
