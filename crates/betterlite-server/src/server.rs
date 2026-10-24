use crate::connection_limit::BoundedListener;
use crate::error_body::request_timeout_error;
use crate::routes::router;
use crate::shutdown::build_shutdown_signal;
use axum::error_handling::HandleErrorLayer;
use betterlite_config::types::Config;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;

const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(1);

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
    let application = router(
        config.general_settings.master_key,
        Arc::clone(&shutting_down),
    )
    .layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
                request_timeout_error()
            }))
            .layer(TimeoutLayer::new(Duration::from_secs(30))),
    );
    let mut listener = BoundedListener::new(tcp_listener);
    let mut connections = JoinSet::new();
    let mut connection_builder = Builder::new(TokioExecutor::new());
    let mut http1 = connection_builder.http1();
    http1.timer(TokioTimer::new());
    http1.header_read_timeout(HEADER_READ_TIMEOUT);
    http1.keep_alive(false);
    let shutdown = shutdown_signal.wait(shutting_down);
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = listener.accept() => {
                let (stream, address) = result.map_err(|source| ServerError::Bind {
                    address: format!("{host}:{port}"),
                    source,
                })?;
                let application = application.clone();
                let builder = connection_builder.clone();
                connections.spawn(async move {
                    let service = TowerToHyperService::new(application);
                    if let Err(error) = builder
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                    {
                        tracing::debug!(%address, %error, "HTTP connection stopped");
                    }
                });
            }
            result = connections.join_next() => {
                if let Some(Err(error)) = result {
                    tracing::error!(%error, "HTTP connection task stopped");
                }
            }
        }
    }

    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            tracing::error!(%error, "HTTP connection task stopped during shutdown");
        }
    }
    Ok(())
}
