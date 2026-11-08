pub mod chat_setup;

use crate::admission_control::in_flight::InFlightRequests;
use crate::admission_control::{AdmissionControl, AdmissionStack};
use crate::auth::auth_route_patterns::AuthSettings;
use crate::middleware::request_root_paths::RootPaths;
use crate::routes::chat::chat_routes_with_call_id;
use crate::routes::embeddings::embedding_routes;
use crate::routes::router;
use crate::shutdown::build_shutdown_signal;
use axum::Router;
use betterlite_config::types::Config;
use betterlite_error::envelope::EnvelopeError;
use betterlite_openai::chat::execution::ChatExecutor;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
const MAX_CONNECTIONS: usize = 1024;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const OUTBOUND_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    ErrorLayer {
        #[from]
        source: EnvelopeError,
    },
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
pub async fn run(
    config: Config,
    chat: ChatExecutor,
    host: &str,
    port: u16,
) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
    let shutting_down = Arc::new(AtomicBool::new(false));
    let outbound = chat.client.clone();
    let application = application(&config, chat, Arc::clone(&shutting_down))?;
    let address = format!("{host}:{port}");
    let listener = TcpListener::bind((host, port))
        .await
        .map_err(|source| ServerError::Bind { address, source })?;
    tracing::info!(host, port, "betterlite server listening");
    let mut connections = JoinSet::new();
    let connection_builder = configure_connection_builder();
    connections.spawn(shutdown_signal.wait(Arc::clone(&shutting_down)));
    while !shutting_down.load(Ordering::Acquire) {
        while let Some(result) = connections.try_join_next() {
            if let Err(error) = result {
                tracing::error!(error = ?error, "HTTP connection task stopped");
            }
        }
        let Some((stream, remote_address)) = accept_connection(&listener).await else {
            continue;
        };
        // The set also holds the shutdown signal task, so this admits MAX_CONNECTIONS connections.
        if connections.len() > MAX_CONNECTIONS {
            tracing::warn!(address = ?remote_address, "HTTP connection limit reached");
            continue;
        }
        connections.spawn(serve_connection(
            connection_builder.clone(),
            application.clone(),
            stream,
            remote_address,
        ));
    }
    drain_connections(&mut connections).await;
    if let Err(error) = outbound.drain(OUTBOUND_DRAIN_TIMEOUT).await {
        tracing::warn!(error = %error, "outbound requests did not finish before shutdown");
    }
    Ok(())
}

pub(crate) fn application(
    config: &Config,
    chat: ChatExecutor,
    shutting_down: Arc<AtomicBool>,
) -> Result<Router, ServerError> {
    let executor = Arc::new(chat);
    let routed = router(
        AuthSettings {
            master_key: config.general_settings.master_key.clone(),
            routes: config.sections.auth_route_patterns.clone(),
        },
        shutting_down,
        &config.model_list,
        chat_routes_with_call_id(
            Arc::clone(&executor),
            config
                .sections
                .error_body_call_id
                .include_call_id_in_error_body,
        )?,
        embedding_routes(executor),
    );
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(
            &config.sections.admission_control,
        )),
        in_flight: Arc::new(InFlightRequests::default()),
        scalar_root: config.sections.request_root_paths.server_root_path.clone(),
    };
    let application = stack.wrap(routed);
    Ok(
        match RootPaths::from_settings(&config.sections.request_root_paths) {
            Some(paths) => paths.wrap(application),
            None => application,
        },
    )
}
async fn accept_connection(listener: &TcpListener) -> Option<(TcpStream, SocketAddr)> {
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

pub mod inputs;
mod tests;
