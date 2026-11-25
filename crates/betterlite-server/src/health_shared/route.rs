use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::AuthContext;
use crate::health_access_and_connections::connection_error::{Rejection, RejectionDetail};
use crate::tracing_routes::router::unauthenticated;

use super::status_view::{SharedHealthStatusError, SharedHealthStatusSource};

pub const SHARED_STATUS_PATH: &str = "/health/shared-status";

// use_shared_health_check: whether the proxy coordinates health checks across pods at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedHealthSwitch {
    Enabled,
    Disabled,
}

// The route's inputs: the proxy's switch and, when Redis is configured, whoever answers the status.
#[derive(Clone)]
pub struct SharedStatusState {
    pub checks: SharedHealthSwitch,
    pub status: Option<Arc<dyn SharedHealthStatusSource>>,
}

// The one route B8540 mounts inside the proxy's authenticated stage.
#[must_use]
pub fn shared_status_router(state: SharedStatusState) -> Router {
    Router::new()
        .route(SHARED_STATUS_PATH, get(shared_status))
        .with_state(state)
}

async fn shared_status(
    State(state): State<SharedStatusState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    if state.checks == SharedHealthSwitch::Disabled {
        return Json(disabled_body()).into_response();
    }
    let Some(source) = state.status.as_deref() else {
        return Json(redis_unavailable_body()).into_response();
    };
    match source.status().await {
        Ok(status) => Json(enabled_body(status)).into_response(),
        Err(error) => status_failure(&error),
    }
}

// The source answers before the switch: a disabled proxy says so and never reads Redis.
fn disabled_body() -> JsonObject {
    let mut body = JsonObject::new();
    body.insert("shared_health_check_enabled".to_owned(), Json::Bool(false));
    body.insert(
        "message".to_owned(),
        Json::Text("Shared health check is not enabled".to_owned()),
    );
    body
}

fn redis_unavailable_body() -> JsonObject {
    let mut body = JsonObject::new();
    body.insert("shared_health_check_enabled".to_owned(), Json::Bool(true));
    body.insert("redis_available".to_owned(), Json::Bool(false));
    body.insert(
        "message".to_owned(),
        Json::Text("Redis is not configured".to_owned()),
    );
    body
}

fn enabled_body(status: JsonObject) -> JsonObject {
    let mut body = JsonObject::new();
    body.insert("shared_health_check_enabled".to_owned(), Json::Bool(true));
    body.insert("status".to_owned(), Json::Object(status));
    body
}

fn status_failure(error: &SharedHealthStatusError) -> Response {
    Rejection {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        detail: RejectionDetail::Error(format!(
            "Failed to retrieve shared health check status: {error}"
        )),
    }
    .into_response()
}
