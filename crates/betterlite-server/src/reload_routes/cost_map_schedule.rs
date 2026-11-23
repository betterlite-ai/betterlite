use axum::Json;
use axum::extract::{Extension, State};
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::caller::{caller_principal, require_proxy_admin};
use super::error::{ReloadError, WrappedCause};
use super::hours::parse_hours;

const SCHEDULE_FAILED: &str = "Failed to schedule model cost map reload";
const CANCEL_FAILED: &str = "Failed to cancel model cost map reload";

#[derive(Serialize)]
struct ScheduleBody {
    message: String,
    status: &'static str,
    interval_hours: u32,
    timestamp: String,
}

#[derive(Serialize)]
struct CancelBody {
    message: &'static str,
    status: &'static str,
    timestamp: String,
}

pub(super) async fn schedule_model_cost_map_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
    uri: Uri,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    let hours = parse_hours(&uri)?;
    require_proxy_admin(&principal)?;
    if hours <= 0 {
        return Err(ReloadError::HoursNotPositive);
    }
    let Some(store) = state.config.as_deref() else {
        return Err(ReloadError::Wrapped {
            prefix: SCHEDULE_FAILED,
            cause: WrappedCause::MissingDatabase,
        });
    };
    let interval_hours = u32::try_from(hours).map_err(|_source| ReloadError::Wrapped {
        prefix: SCHEDULE_FAILED,
        cause: WrappedCause::Interval { hours },
    })?;
    store
        .write_interval(interval_hours)
        .await
        .map_err(|source| ReloadError::Wrapped {
            prefix: SCHEDULE_FAILED,
            cause: WrappedCause::Store(source),
        })?;
    let now = (state.clock)();
    let body = ScheduleBody {
        message: format!("Model cost map reload scheduled for every {interval_hours} hours"),
        status: "success",
        interval_hours,
        timestamp: now.to_rfc3339(),
    };
    Ok(Json(body).into_response())
}

pub(super) async fn cancel_model_cost_map_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_proxy_admin(&principal)?;
    let Some(store) = state.config.as_deref() else {
        return Err(ReloadError::Wrapped {
            prefix: CANCEL_FAILED,
            cause: WrappedCause::MissingDatabase,
        });
    };
    store
        .clear_interval()
        .await
        .map_err(|source| ReloadError::Wrapped {
            prefix: CANCEL_FAILED,
            cause: WrappedCause::Store(source),
        })?;
    let now = (state.clock)();
    let body = CancelBody {
        message: "Model cost map reload schedule cancelled",
        status: "success",
        timestamp: now.to_rfc3339(),
    };
    Ok(Json(body).into_response())
}
