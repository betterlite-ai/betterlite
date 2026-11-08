pub(crate) mod agent_daily_activity;
pub(crate) mod assistant_runs;
pub(crate) mod chat;
pub(crate) mod completions;
pub(crate) mod container_files;
pub(crate) mod containers;
pub(crate) mod embeddings;
pub(crate) mod mcp_byok_consent;
pub(crate) mod mcp_byok_oauth;
pub(crate) mod moderations;
pub(crate) mod ocr;
pub(crate) mod prompts;
pub(crate) mod video_character;
pub(crate) mod video_content_remix;
pub(crate) mod video_create;
pub(crate) mod video_edit;

use crate::auth::auth_route_patterns::{self, AuthSettings};
use crate::auth::route_match::stage;
use crate::error_body::json_error;
use crate::model_list::model_routes;
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_config::types::ModelDeployment;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
pub(crate) fn router(
    auth: AuthSettings,
    shutting_down: Arc<AtomicBool>,
    model_list: &[ModelDeployment],
    chat: Router,
    embeddings: Router,
) -> Router {
    let timeout_layers = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
            json_error(
                StatusCode::REQUEST_TIMEOUT,
                "request timed out",
                "invalid_request_error",
                "408",
            )
        }))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));
    let application = Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .route("/health/readiness", get(health_readiness))
        .with_state(shutting_down)
        .merge(model_routes(model_list))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(NotFoundBody {
                    detail: "Not Found",
                }),
            )
        })
        .layer(timeout_layers)
        .merge(chat)
        .merge(embeddings);
    application
        .layer(middleware::from_fn_with_state(
            auth,
            auth_route_patterns::stage,
        ))
        .layer(middleware::from_fn(stage))
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
