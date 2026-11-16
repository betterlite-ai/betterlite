use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

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

pub(crate) fn configure_connection_builder() -> Builder<TokioExecutor> {
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    let mut http1 = builder.http1();
    http1.timer(TokioTimer::new());
    http1.header_read_timeout(HEADER_READ_TIMEOUT);
    http1.keep_alive(false);
    builder
}
pub(crate) async fn serve_connection(
    builder: Builder<TokioExecutor>,
    application: Router,
    stream: TcpStream,
    remote_address: SocketAddr,
) {
    let service = TowerToHyperService::new(application);
    if let Err(error) = builder
        .serve_connection(TokioIo::new(stream), service)
        .await
    {
        tracing::debug!(address = ?remote_address, error = ?error, "HTTP connection stopped");
    }
}
