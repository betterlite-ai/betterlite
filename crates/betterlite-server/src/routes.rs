use crate::auth::{AuthState, authenticate};
use crate::error_body::not_found_error;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use secrecy::SecretString;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
pub(crate) fn router(master_key: Option<SecretString>, shutting_down: Arc<AtomicBool>) -> Router {
    let state = AuthState { master_key };
    Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .fallback(not_found_error)
        .with_state(shutting_down)
        .layer(middleware::from_fn_with_state(state, authenticate))
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
