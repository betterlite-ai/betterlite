pub(crate) mod chat;

use crate::auth::authenticate;
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
use secrecy::SecretString;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
pub(crate) fn router(
    master_key: Option<SecretString>,
    shutting_down: Arc<AtomicBool>,
    model_list: &[ModelDeployment],
    chat: Router,
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
    // the deployment timeout bounds a chat call, and LiteLLM's default is 6000 seconds
    let guarded_chat = chat.layer(middleware::from_fn_with_state(
        master_key.clone(),
        authenticate,
    ));
    Router::new()
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
        .layer(middleware::from_fn_with_state(master_key, authenticate))
        .layer(timeout_layers)
        .merge(guarded_chat)
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
