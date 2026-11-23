use axum::Json;
use axum::extract::{Extension, State};
use axum::http::Uri;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::caller::{caller_principal, require_proxy_admin, require_write_admin, write_store};
use super::error::{ReloadError, WrappedCause, store_call};
use super::hours::parse_hours;

const SCHEDULE_FAILED: &str = "Failed to schedule model cost map reload";
const CANCEL_FAILED: &str = "Failed to cancel model cost map reload";

#[derive(Serialize)]
pub(super) struct ScheduleBody {
    message: String,
    status: &'static str,
    interval_hours: u32,
    timestamp: String,
}

#[derive(Serialize)]
pub(super) struct CancelBody {
    message: &'static str,
    status: &'static str,
    timestamp: String,
}

pub(super) async fn schedule_model_cost_map_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
    uri: Uri,
) -> Result<Json<ScheduleBody>, ReloadError> {
    let principal = caller_principal(caller)?;
    let hours = parse_hours(&uri)?;
    require_proxy_admin(&principal)?;
    if hours <= 0 {
        return Err(ReloadError::HoursNotPositive);
    }
    let interval_hours = u32::try_from(hours).map_err(|_source| ReloadError::Wrapped {
        prefix: SCHEDULE_FAILED,
        cause: WrappedCause::Interval { hours },
    })?;
    let store = write_store(&state, SCHEDULE_FAILED)?;
    store_call(store.write_interval(interval_hours), SCHEDULE_FAILED).await?;
    let now = (state.clock)();
    Ok(Json(ScheduleBody {
        message: format!("Model cost map reload scheduled for every {interval_hours} hours"),
        status: "success",
        interval_hours,
        timestamp: now.to_rfc3339(),
    }))
}

pub(super) async fn cancel_model_cost_map_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<CancelBody>, ReloadError> {
    require_write_admin(caller)?;
    let store = write_store(&state, CANCEL_FAILED)?;
    store_call(store.clear_interval(), CANCEL_FAILED).await?;
    let now = (state.clock)();
    Ok(Json(CancelBody {
        message: "Model cost map reload schedule cancelled",
        status: "success",
        timestamp: now.to_rfc3339(),
    }))
}
