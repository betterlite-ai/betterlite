use std::sync::Arc;

use axum::Json as AxumJson;
use axum::Router;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::health_access_and_connections::connection_error::{Rejection, RejectionDetail};
use crate::tracing_routes::router::unauthenticated;

use super::status_view::{SharedHealthStatus, SharedHealthStatusError, SharedHealthStatusSource};

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

// The two answers that stop before the status source: a disabled switch and a proxy without Redis.
#[derive(Serialize)]
struct NotServedStatus {
    shared_health_check_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    redis_available: Option<bool>,
    message: &'static str,
}

// The enabled answer: LiteLLM's flag around get_health_check_status' dict.
#[derive(Serialize)]
struct ServedStatus {
    shared_health_check_enabled: bool,
    status: SharedHealthStatus,
}

// The one route B8540 mounts inside the proxy's authenticated stage.
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
        return AxumJson(NotServedStatus {
            shared_health_check_enabled: false,
            redis_available: None,
            message: "Shared health check is not enabled",
        })
        .into_response();
    }
    let Some(source) = state.status.as_deref() else {
        return AxumJson(NotServedStatus {
            shared_health_check_enabled: true,
            redis_available: Some(false),
            message: "Redis is not configured",
        })
        .into_response();
    };
    match source.status().await {
        Ok(status) => AxumJson(ServedStatus {
            shared_health_check_enabled: true,
            status,
        })
        .into_response(),
        Err(error) => status_failure(&error),
    }
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
