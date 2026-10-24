use crate::auth::authenticate;
use crate::error_body::request_timeout_error;
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use secrecy::SecretString;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
pub(crate) fn router(master_key: Option<SecretString>, shutting_down: Arc<AtomicBool>) -> Router {
    let timeout_layers = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
            request_timeout_error()
        }))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));
    Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(NotFoundBody {
                    detail: "Not Found",
                }),
            )
        })
        .with_state(shutting_down)
        .layer(middleware::from_fn_with_state(master_key, authenticate))
        .layer(timeout_layers)
}
async fn health_liveliness(State(shutting_down): State<Arc<AtomicBool>>) -> Response {
    if shutting_down.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ShutdownBody {
                status: "shutting_down",
            }),
        )
            .into_response();
    }
    Json("I'm alive!").into_response()
}
#[derive(Serialize)]
struct ShutdownBody {
    status: &'static str,
}
#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}
