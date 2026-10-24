use crate::routes::router;
use crate::shutdown::{ShutdownSignal, build_shutdown_signal};
use axum::Router;
use axum::body::Body;
use axum::error_handling::HandleErrorLayer;
use axum::http::{Request, StatusCode};
use betterlite_config::types::Config;
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
use tokio::task::{JoinError, JoinSet};
use tokio::time::timeout;
use tower::ServiceBuilder;
use tower::ServiceExt as _;
use tower::timeout::TimeoutLayer;
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
    let graceful_shutdown = GracefulShutdown::new();
    let application = router(
        config.general_settings.master_key,
        Arc::clone(&shutting_down),
    )
    .layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
                (StatusCode::REQUEST_TIMEOUT, "request timed out")
            }))
            .layer(TimeoutLayer::new(Duration::from_secs(30))),
    );
    serve_connections(
        listener,
        application,
        graceful_shutdown,
        shutdown_signal,
        shutting_down,
    )
    .await
}
async fn serve_connections(
    listener: TcpListener,
    application: Router,
    graceful_shutdown: GracefulShutdown,
    signal: ShutdownSignal,
    shutting_down: Arc<AtomicBool>,
) -> Result<(), ServerError> {
    let mut signals: JoinSet<()> = JoinSet::new();
    signals.spawn(async move {
        signal.wait(shutting_down).await;
    });
    let mut connections = JoinSet::new();
    accept_connections(
        listener,
        &application,
        &graceful_shutdown,
        &mut signals,
        &mut connections,
    )
    .await?;
    timeout(Duration::from_secs(30), graceful_shutdown.shutdown())
        .await
        .unwrap_or(());
    signals.shutdown().await;
    while let Some(result) = connections.join_next().await {
        result.map_err(ServerError::Connection)?;
    }
    Ok(())
}
async fn accept_connections(
    listener: TcpListener,
    application: &Router,
    graceful_shutdown: &GracefulShutdown,
    signals: &mut JoinSet<()>,
    connections: &mut JoinSet<()>,
) -> Result<(), ServerError> {
    loop {
        let shutdown_requested = if let Some(result) = signals.try_join_next() {
            result.map_err(ServerError::Connection)?;
            true
        } else {
            false
        };
        if shutdown_requested {
            if let Ok(accepted) = timeout(Duration::from_millis(1), listener.accept()).await {
                let (socket, _) = accepted.map_err(ServerError::Accept)?;
                let cloned_application = application.clone();
                let watcher = graceful_shutdown.watcher();
                connections.spawn(serve_connection(socket, cloned_application, watcher));
            }
            break;
        }
        match timeout(Duration::from_millis(100), listener.accept()).await {
            Ok(Ok((socket, _))) => {
                let cloned_application = application.clone();
                let watcher = graceful_shutdown.watcher();
                connections.spawn(serve_connection(socket, cloned_application, watcher));
            }
            Ok(Err(error)) => {
                signals.shutdown().await;
                return Err(ServerError::Accept(error));
            }
            Err(_) => {}
        }
        while let Some(result) = connections.try_join_next() {
            if let Err(error) = result {
                signals.shutdown().await;
                return Err(ServerError::Connection(error));
            }
        }
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
        .header_read_timeout(Duration::from_secs(1));
    let connection = builder.serve_connection(io, service);
    if let Err(error) = watcher.watch(connection).await {
        tracing::debug!(error = %error, "HTTP connection stopped");
    }
}
