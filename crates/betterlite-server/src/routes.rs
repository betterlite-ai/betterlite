use crate::auth::authenticate;
use crate::error_body::request_timeout_error;
use crate::model_list::model_routes;
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::routing::post;
use betterlite_config::types::ModelDeployment;
use betterlite_http::outbound::OutboundClient;
use betterlite_openai::chat::{ChatError, ChatExecutor, ChatRequest};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use secrecy::SecretString;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
pub(crate) fn router(
    master_key: Option<SecretString>,
    shutting_down: Arc<AtomicBool>,
    model_list: &[ModelDeployment],
) -> Router {
    let timeout_layers = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
            request_timeout_error()
        }))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));
    Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .route("/health/readiness", get(health_readiness))
        .with_state(shutting_down)
        .merge(model_routes(model_list))
        .merge(chat_routes(model_list))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(NotFoundBody {
                    detail: "Not Found",
                }),
            )
        })
        .layer(middleware::from_fn_with_state(master_key, authenticate))
        .layer(timeout_layers)
}

fn chat_routes(model_list: &[ModelDeployment]) -> Router {
    let executor = OutboundClient::new().ok().map(|client| {
        ChatExecutor::new(
            client,
            None,
            CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
        )
    });
    let state = ChatState {
        deployments: Arc::from(model_list.to_vec()),
        executor,
    };
    Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        )
        .with_state(Arc::new(state))
}

struct ChatState {
    deployments: Arc<[ModelDeployment]>,
    executor: Option<ChatExecutor>,
}

async fn chat_completion(
    State(state): State<Arc<ChatState>>,
    Json(request): Json<ChatRequest>,
) -> Response {
    let Some(executor) = &state.executor else {
        return chat_error(&ChatError::UnknownModel);
    };
    match executor.complete(&request, &state.deployments, None).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => chat_error(&error),
    }
}

fn chat_error(error: &ChatError) -> Response {
    let status = match error {
        ChatError::UnknownModel => StatusCode::NOT_FOUND,
        ChatError::UnsupportedProvider => StatusCode::NOT_IMPLEMENTED,
        ChatError::Timeout => StatusCode::REQUEST_TIMEOUT,
        ChatError::Provider { status, .. } => {
            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
        }
        ChatError::Url { .. } | ChatError::Decode { .. } | ChatError::Http { .. } => {
            StatusCode::BAD_GATEWAY
        }
    };
    let body = ChatErrorBody {
        error: ChatErrorObject {
            message: error.to_string(),
            error_type: "invalid_request_error".to_owned(),
            param: None,
            code: status.as_u16().to_string(),
        },
    };
    (status, Json(body)).into_response()
}

#[derive(Serialize)]
struct ChatErrorBody {
    error: ChatErrorObject,
}

#[derive(Serialize)]
struct ChatErrorObject {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}
async fn health_liveliness(State(shutting_down): State<Arc<AtomicBool>>) -> Response {
    if shutting_down.load(Ordering::Acquire) {
        return shutdown_health_response();
    }
    Json("I'm alive!").into_response()
}
async fn health_readiness(State(shutting_down): State<Arc<AtomicBool>>) -> Response {
    if shutting_down.load(Ordering::Acquire) {
        return shutdown_health_response();
    }
    Json(ReadinessBody { status: "healthy" }).into_response()
}
fn shutdown_health_response() -> Response {
    let body = ShutdownBody {
        status: "shutting_down",
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}
#[derive(Serialize)]
struct ShutdownBody {
    status: &'static str,
}
#[derive(Serialize)]
struct ReadinessBody {
    status: &'static str,
}
#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}
