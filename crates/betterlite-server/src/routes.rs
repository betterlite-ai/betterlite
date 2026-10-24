use axum::Json;
use axum::Router;
use axum::middleware;
use axum::routing::get;
use secrecy::SecretString;

use crate::auth::{AuthState, authenticate};
use crate::error_body::not_found_error;

pub(crate) fn router(master_key: Option<SecretString>) -> Router {
    let state = AuthState { master_key };
    Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .fallback(not_found_error)
        .layer(middleware::from_fn_with_state(state, authenticate))
}

async fn health_liveliness() -> Json<&'static str> {
    let body = "I'm alive!";
    Json(body)
}
