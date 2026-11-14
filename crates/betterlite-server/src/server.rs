pub mod chat_setup;

use crate::admission_control::in_flight::InFlightRequests;
use crate::admission_control::{AdmissionControl, AdmissionStack};
use crate::middleware::request_root_paths::RootPaths;
use crate::middleware::request_stack;
use crate::routes::chat::CallContext;
use crate::routes::chat::chat_routes_with_call_id;
use crate::routes::chat::headers::failure_headers;
use crate::routes::embeddings::embedding_routes;
use crate::routes::router;
use crate::server::filter::filter_route;
use crate::shutdown::build_shutdown_signal;
use crate::state::ServerState;
use crate::state::shared::Shared;
use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::response::{IntoResponse as _, Response as AxumResponse};
use axum::{Json, Router};
use betterlite_config::types::Config;
use betterlite_error::envelope::EnvelopeError;
use betterlite_error::envelope::error_body_call_id;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;
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
    let shutting_down = Arc::new(AtomicBool::new(false));
    let outbound = chat.client.clone();
    let application = application(&config, chat, Arc::clone(&shutting_down))?;
    Box::pin(serve(application, shutting_down, outbound, host, port)).await
}

async fn serve(
    application: Router,
    shutting_down: Arc<AtomicBool>,
    outbound: ProxiedClient,
    host: &str,
    port: u16,
) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
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
    let shared = Arc::new(Shared);
    let state = Arc::new(ServerState::new(config, Arc::clone(&executor), shared));
    let routed = router(
        state,
        shutting_down,
        chat_routes_with_call_id(
            Arc::clone(&executor),
            config
                .sections
                .error_body_call_id
                .include_call_id_in_error_body,
        )?,
        embedding_routes(executor),
    );
    // a route layer, so a blocked route's 404 still passes the security headers, size limit and admission below
    let stacked = request_stack::wrap(
        routed.route_layer(from_fn(filter_route)),
        &config.sections.request_size_limit,
        &config.sections.security_headers,
    );
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(
            &config.sections.admission_control,
        )),
        in_flight: Arc::new(InFlightRequests::default()),
    };
    let application = stack.wrap(stacked);
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

// the OpenAI-shaped error body every route renders, and the projections that fill it
#[derive(serde::Serialize)]
pub(crate) struct ErrorBody {
    pub(crate) error: ErrorObject,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorObject {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) error_type: String,
    pub(crate) param: Option<String>,
    pub(crate) code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_specific_fields: Option<RouterErrorFields>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) litellm_call_id: Option<String>,
}

// LiteLLM's router repeats its message here when no deployment matches
#[derive(serde::Serialize)]
pub(crate) struct RouterErrorFields {
    pub(crate) error: String,
}

pub(crate) fn plain_failure(
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
    status: StatusCode,
    kind: (&str, Option<String>),
    message: String,
) -> AxumResponse {
    let (error_type, param) = kind;
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: error_type.to_owned(),
            param,
            code: status.as_u16().to_string(),
            provider_specific_fields: None,
            litellm_call_id: error_body_call_id(call.include_in_body, Some(call.id))
                .map(str::to_owned),
        },
    };
    let headers = failure_headers(None, None, call.id, deployment);
    (status, headers, Json(body)).into_response()
}

pub(crate) fn failure(
    error: &ChatError,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> AxumResponse {
    let failure = error.failure(&call.errors.redactor);
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let redactor = &call.errors.redactor;
    let message = redactor.redact_internal_details(&error.to_string());
    call.errors
        .log_normalized_error(error, &message, status.as_u16());
    let provider_specific_fields =
        matches!(error, ChatError::UnknownModel { .. }).then(|| RouterErrorFields {
            error: message.clone(),
        });
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: redactor.redact_internal_details(&failure.error_type),
            param: failure
                .param
                .map(|param| redactor.redact_internal_details(&param)),
            code: status.as_u16().to_string(),
            provider_specific_fields,
            litellm_call_id: error_body_call_id(call.include_in_body, Some(call.id))
                .map(str::to_owned),
        },
    };
    let headers = failure_headers(Some(error), None, call.id, deployment);
    (status, headers, Json(body)).into_response()
}

pub mod component;
pub mod filter;
pub mod inputs;
pub mod listener;
mod tests;
