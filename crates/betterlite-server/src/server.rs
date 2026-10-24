use crate::connection_limit::{BoundedListener, BoundedStream, MAX_CONNECTIONS};
use crate::error_body::request_timeout_error;
use crate::routes::router;
use crate::shutdown::build_shutdown_signal;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use betterlite_config::types::Config;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to register {signal} handler: {source}")]
    SignalRegistration {
        signal: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("failed to bind {address}: {source}")]
    Bind {
        address: String,
        #[source]
        source: io::Error,
    },
}
pub async fn run(config: Config, host: &str, port: u16) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
    let address = format!("{host}:{port}");
    let tcp_listener = TcpListener::bind((host, port))
        .await
        .map_err(|source| ServerError::Bind { address, source })?;
    tracing::info!(host, port, "betterlite server listening");
    let shutting_down = Arc::new(AtomicBool::new(false));
    let application = build_application(config, Arc::clone(&shutting_down));
    let listener = BoundedListener {
        listener: tcp_listener,
        permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
    };
    let mut connections = JoinSet::new();
    let connection_builder = configure_connection_builder();
    connections.spawn(shutdown_signal.wait(Arc::clone(&shutting_down)));
    while !shutting_down.load(Ordering::Acquire) {
        if let Some((stream, remote_address)) = accept_connection(&listener).await {
            connections.spawn(serve_connection(
                connection_builder.clone(),
                application.clone(),
                stream,
                remote_address,
            ));
        }
        if let Some(Err(error)) = connections.try_join_next() {
            tracing::error!(error = ?error, "HTTP connection task stopped");
        }
    }
    drain_connections(&mut connections).await;
    Ok(())
}
async fn accept_connection(listener: &BoundedListener) -> Option<(BoundedStream, SocketAddr)> {
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
fn build_application(config: Config, shutting_down: Arc<AtomicBool>) -> Router {
    let timeout_layers = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
            request_timeout_error()
        }))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));
    router(config.general_settings.master_key, shutting_down).layer(timeout_layers)
}
fn configure_connection_builder() -> Builder<TokioExecutor> {
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    let mut http1 = builder.http1();
    http1.timer(TokioTimer::new());
    http1.header_read_timeout(HEADER_READ_TIMEOUT);
    http1.keep_alive(false);
    builder
}
async fn serve_connection(
    builder: Builder<TokioExecutor>,
    application: Router,
    stream: BoundedStream,
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
async fn drain_connections(connections: &mut JoinSet<()>) {
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
