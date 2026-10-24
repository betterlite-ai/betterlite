use crate::routes::router;
use axum::Router;
use axum::body::Body;
use axum::error_handling::HandleErrorLayer;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::types::Config;
use futures_util::future::{Either, select};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::watch;
use tokio::task::JoinSet;
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
    #[error("server failed: {0}")]
    Serve(#[source] io::Error),
    #[error("shutdown signal channel failed: {0}")]
    Shutdown(#[source] watch::error::RecvError),
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
    let server = serve_connections(listener, application, shutdown_receiver);
    tokio::pin!(server);
    let shutdown_future = shutdown_signal.wait(Arc::clone(&shutting_down), shutdown_sender);
    tokio::pin!(shutdown_future);
    match select(server, shutdown_future).await {
        Either::Left((result, _)) => result.map_err(ServerError::Serve),
        Either::Right(((), server_future)) => timeout(DRAIN_TIMEOUT, server_future)
            .await
            .map_or(Ok(()), |result| result.map_err(ServerError::Serve)),
    }
}
async fn serve_connections(
    listener: TcpListener,
    application: Router,
    mut shutdown_receiver: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        let accepted = listener.accept();
        let changed = shutdown_receiver.changed();
        tokio::pin!(accepted);
        tokio::pin!(changed);
        match select(accepted, changed).await {
            Either::Left((accepted_result, _)) => {
                let (socket, _) = accepted_result?;
                let cloned_application = application.clone();
                connections.spawn(serve_connection(socket, cloned_application));
            }
            Either::Right((changed_result, _)) => {
                changed_result.map_err(io::Error::other)?;
                break;
            }
        }
    }
    while let Some(result) = connections.join_next().await {
        result.map_err(io::Error::other)?;
    }
    Ok(())
}
async fn serve_connection(socket: TcpStream, application: Router) {
    let io = TokioIo::new(socket);
    let service = TowerToHyperService::new(
        application.map_request(|request: Request<Incoming>| request.map(Body::new)),
    );
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    if let Err(error) = builder.serve_connection_with_upgrades(io, service).await {
        tracing::debug!(error = %error, "HTTP connection stopped");
    }
}
async fn request_timeout_error(_: tower::BoxError) -> Response {
    let status = StatusCode::REQUEST_TIMEOUT;
    (status, "request timed out").into_response()
}
struct ShutdownSignal {
    #[cfg(unix)]
    terminate: Signal,
    #[cfg(unix)]
    interrupt: Signal,
}
fn build_shutdown_signal() -> Result<ShutdownSignal, ServerError> {
    #[cfg(unix)]
    {
        let terminate =
            signal(SignalKind::terminate()).map_err(|source| ServerError::SignalRegistration {
                signal: "SIGTERM",
                source,
            })?;
        let interrupt =
            signal(SignalKind::interrupt()).map_err(|source| ServerError::SignalRegistration {
                signal: "SIGINT",
                source,
            })?;
        Ok(ShutdownSignal {
            terminate,
            interrupt,
        })
    }
    #[cfg(not(unix))]
    {
        Ok(ShutdownSignal {})
    }
}
impl ShutdownSignal {
    async fn wait(mut self, shutting_down: Arc<AtomicBool>, sender: watch::Sender<bool>) {
        #[cfg(unix)]
        {
            let terminate = self.terminate.recv();
            let interrupt = self.interrupt.recv();
            tokio::pin!(terminate);
            tokio::pin!(interrupt);
            match select(terminate, interrupt).await {
                Either::Left((_result, _)) | Either::Right((_result, _)) => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        shutting_down.store(true, Ordering::Release);
        let _send_result: Result<(), _> = sender.send(true);
    }
}
