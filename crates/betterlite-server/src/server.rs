pub mod chat_setup;

use crate::admission_control::in_flight::InFlightRequests;
use crate::admission_control::{AdmissionControl, AdmissionStack};
use crate::app::inference::native_compaction;
use crate::middleware::request_root_paths::RootPaths;
use crate::middleware::request_stack;
use crate::pre_call::rewrites::resolve_call_id;
use crate::routes::chat::CallContext;
use crate::routes::chat::chat_routes_with_call_id;
use crate::routes::chat::headers::failure_headers;
use crate::routes::chat::streaming::ChatCalls;
use crate::routes::embeddings::embedding_routes;
use crate::routes::router;
use crate::server::connections::{
    Background, accept_connection, configure_connection_builder, drain_connections,
    serve_connection,
};
use crate::server::filter::filter_route;
use crate::shutdown::build_shutdown_signal;
use crate::state::ServerState;
use crate::state::shared::Shared;
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse as _, Response as AxumResponse};
use axum::{Extension, Json, Router};
use betterlite_config::types::Config;
use betterlite_error::envelope::EnvelopeError;
use betterlite_error::envelope::error_body_call_id;
use betterlite_logging::logging_service_events::clock::SystemClock;
use betterlite_logging::logging_service_events::middleware::{ServiceEventState, service_events};
use betterlite_logging::logging_service_events::pipeline::PipelineError;
use betterlite_logging::logging_service_events::sink::TracingSink;
use betterlite_logging::logging_service_events::wiring::LoggingWiring;
use betterlite_logging::logging_streams::assembler::ChatStreamAssembler;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::chat::execution::ChatExecutor;
use chat_setup::vertex_embedding::{VertexEmbeddingSetupError, vertex_embedding_calls};
use error::ChatCallsError;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;
const MAX_CONNECTIONS: usize = 1024;
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
    #[error("failed to read the listening address: {source}")]
    ListenerAddress {
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    ChatCalls {
        #[from]
        source: ChatCallsError,
    },
    #[error(transparent)]
    VertexEmbedding(#[from] VertexEmbeddingSetupError),
    #[error(transparent)]
    Logging(#[from] PipelineError),
}
pub async fn run(
    config: Config,
    chat: ChatExecutor,
    host: &str,
    port: u16,
) -> Result<(), ServerError> {
    let listener = bind(host, port).await?;
    // a taken port stops startup before the application is built
    Box::pin(run_with_listener(config, chat, listener, host)).await
}

// tests bind and keep their own listener, so no other test can take its port before the server serves on it
pub async fn run_with_listener(
    config: Config,
    chat: ChatExecutor,
    listener: TcpListener,
    host: &str,
) -> Result<(), ServerError> {
    let shutting_down = Arc::new(AtomicBool::new(false));
    let outbound = chat.client.clone();
    let tasks = TaskTracker::new();
    // the one place the logging parts are built, with the stream assembler B8610 later replaces
    let logging = LoggingWiring::start(
        &config.sections.service_callback,
        Arc::new(SystemClock),
        Arc::new(TracingSink),
        CallbackRegistry::new(config.sections.callback_settings.max_callbacks),
        Arc::new(ChatStreamAssembler),
    )?;
    let events = ServiceEventState {
        pipeline: Arc::clone(&logging.pipeline),
        call_id: resolve_call_id,
    };
    let app = application(&config, chat, Arc::clone(&shutting_down), &tasks)?
        .layer(from_fn_with_state(events, service_events));
    let background = Background { outbound, tasks };
    let served = Box::pin(serve(app, shutting_down, background, host, listener)).await;
    // every accepted service event is delivered before the process reports the shutdown
    logging.pipeline.stop().await;
    served
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, ServerError> {
    let address = format!("{host}:{port}");
    TcpListener::bind((host, port))
        .await
        .map_err(|source| ServerError::Bind { address, source })
}

async fn serve(
    application: Router,
    shutting_down: Arc<AtomicBool>,
    background: Background,
    host: &str,
    listener: TcpListener,
) -> Result<(), ServerError> {
    let shutdown_signal = build_shutdown_signal()?;
    // a server started with port 0 listens on the port the kernel chose
    let bound = listener
        .local_addr()
        .map_err(|source| ServerError::ListenerAddress { source })?;
    tracing::info!(host, port = bound.port(), "betterlite server listening");
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
    background.drain().await;
    Ok(())
}

pub(crate) fn application(
    config: &Config,
    chat: ChatExecutor,
    shutting_down: Arc<AtomicBool>,
    tasks: &TaskTracker,
) -> Result<Router, ServerError> {
    let executor = Arc::new(chat);
    let shared = Arc::new(Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
        client: executor.client.clone(),
    });
    let state = Arc::new(ServerState::new(config, Arc::clone(&executor), shared));
    let slot = &state.anthropic_runtime;
    let azure = &state.azure_runtime;
    let calls = Arc::new(ChatCalls::new(config, Arc::clone(&executor), slot, azure)?);
    let counter = Arc::clone(&calls.counter);
    let vertex = vertex_embedding_calls(config, &executor, tasks)?;
    let read_through = state.model_read_through.read_through.clone();
    let include_call_id = config
        .sections
        .error_body_call_id
        .include_call_id_in_error_body;
    let endpoints = chat_routes_with_call_id(calls, include_call_id)?;
    let embeddings = embedding_routes(executor, vertex, read_through, &azure.composition);
    // the completion route recovers an output-limit 400 with the counter the chat route counts prompts with
    let routed = router(state, shutting_down, endpoints, embeddings).layer(Extension(counter));
    // a route layer, so a blocked route's 404 still passes the security headers, size limit and admission below
    let stacked = request_stack::wrap(
        routed.route_layer(from_fn(filter_route)),
        &config.sections.request_size_limit,
        &config.sections.security_headers,
    );
    let admission = AdmissionControl::from_settings(&config.sections.admission_control);
    let stack = AdmissionStack {
        admission: Arc::new(admission),
        in_flight: Arc::new(InFlightRequests::default()),
    };
    let application = stack.wrap(stacked);
    let prefixed = RootPaths::from_settings(&config.sections.request_root_paths);
    let root_wrapped = match prefixed {
        Some(paths) => paths.wrap(application),
        None => application,
    };
    let scalar_root = config.sections.request_root_paths.server_root_path.clone();
    Ok(native_compaction::install(root_wrapped, scalar_root))
}
// the OpenAI-shaped error body every route renders, and the projections that fill it
#[derive(serde::Serialize)]
pub(crate) struct ErrorBody {
    pub(crate) error: ErrorObject,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorObject {
    pub(crate) message: String,
    // a provider exception with no type of its own travels as null, which the completion route hands on
    #[serde(rename = "type")]
    pub(crate) error_type: Option<String>,
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
            error_type: Some(redactor.redact_internal_details(&failure.error_type)),
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
pub(crate) mod connections;
pub mod error;
pub mod filter;
pub mod inputs;
pub mod listener;
pub(crate) mod render;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;
mod tests;
