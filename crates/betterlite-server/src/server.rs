use crate::routes::router;
use crate::shutdown::build_shutdown_signal;
use axum::Router;
use axum::body::Body;
use axum::error_handling::HandleErrorLayer;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::types::Config;
use futures_util::future::{Either, select};
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::{GracefulShutdown, Watcher};
use hyper_util::service::TowerToHyperService;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::timeout;
use tower::ServiceBuilder;
use tower::ServiceExt as _;
use tower::timeout::TimeoutLayer;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(1);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
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
    #[error("failed to accept connection: {0}")]
    Accept(#[source] io::Error),
    #[error("shutdown signal channel failed: {0}")]
    Shutdown(#[source] watch::error::RecvError),
    #[error("connection task failed: {0}")]
    Connection(#[source] JoinError),
}
pub async fn run(config: Config, host: &str, port: u16) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
    let address = format!("{host}:{port}");
    let listener = TcpListener::bind((host, port))
        .await
        .map_err(|source| ServerError::Bind { address, source })?;
    tracing::info!(host, port, "betterlite server listening");
    let shutting_down = Arc::new(AtomicBool::new(false));
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let graceful_shutdown = GracefulShutdown::new();
    let application = Router::new()
        .merge(router(
            config.general_settings.master_key,
            Arc::clone(&shutting_down),
        ))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(request_timeout_error))
                .layer(TimeoutLayer::new(REQUEST_TIMEOUT)),
        );
    let server = serve_connections(listener, application, shutdown_receiver, graceful_shutdown);
    tokio::pin!(server);
    let shutdown_future = shutdown_signal.wait(Arc::clone(&shutting_down), shutdown_sender);
    tokio::pin!(shutdown_future);
    match select(server, shutdown_future).await {
        Either::Left((result, _)) => result,
        Either::Right(((), server_future)) => timeout(DRAIN_TIMEOUT, server_future)
            .await
            .unwrap_or(Ok(())),
    }
}
async fn serve_connections(
    listener: TcpListener,
    application: Router,
    mut shutdown_receiver: watch::Receiver<bool>,
    graceful_shutdown: GracefulShutdown,
) -> Result<(), ServerError> {
    let mut connections = JoinSet::new();
    loop {
        let accepted = listener.accept();
        let changed = shutdown_receiver.changed();
        tokio::pin!(accepted);
        tokio::pin!(changed);
        match select(accepted, changed).await {
            Either::Left((accepted_result, _)) => {
                let (socket, _) = accepted_result.map_err(ServerError::Accept)?;
                let cloned_application = application.clone();
                let watcher = graceful_shutdown.watcher();
                connections.spawn(serve_connection(socket, cloned_application, watcher));
            }
            Either::Right((changed_result, _)) => {
                changed_result.map_err(ServerError::Shutdown)?;
                break;
            }
        }
    }
    graceful_shutdown.shutdown().await;
    while let Some(result) = connections.join_next().await {
        result.map_err(ServerError::Connection)?;
    }
    Ok(())
}
async fn serve_connection(socket: TcpStream, application: Router, watcher: Watcher) {
    let io = TokioIo::new(socket);
    let service = TowerToHyperService::new(
        application.map_request(|request: Request<Incoming>| request.map(Body::new)),
    );
    let mut builder = Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    let connection = builder.serve_connection(io, service);
    if let Err(error) = watcher.watch(connection).await {
        tracing::debug!(error = %error, "HTTP connection stopped");
    }
}
async fn request_timeout_error(_: tower::BoxError) -> Response {
    let status = StatusCode::REQUEST_TIMEOUT;
    (status, "request timed out").into_response()
}
