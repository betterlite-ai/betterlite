use axum::Json;
use axum::extract::{Extension, State};
use axum::http::Uri;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::beta_config::reload_config_value;
use super::caller::{caller_principal, require_proxy_admin, require_write_admin, write_store};
use super::error::{ReloadError, store_call};
use super::hours::parse_hours;
use super::wire::naive_utc_isoformat;

const SCHEDULE_FAILED: &str = "Failed to schedule anthropic beta headers reload";
const CANCEL_FAILED: &str = "Failed to cancel anthropic beta headers reload";

#[derive(Serialize)]
pub(super) struct ScheduleBody {
    message: String,
    status: &'static str,
    interval_hours: i64,
    timestamp: String,
}

#[derive(Serialize)]
pub(super) struct CancelBody {
    message: &'static str,
    status: &'static str,
    timestamp: String,
}

pub(super) async fn schedule_beta_headers_reload(
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
    let store = write_store(&state, SCHEDULE_FAILED)?;
    let value = reload_config_value(Some(hours), false);
    store_call(store.write_beta_config(value), SCHEDULE_FAILED).await?;
    store_call(store.invalidate_beta_config(), SCHEDULE_FAILED).await?;
    let now = (state.clock)();
    Ok(Json(ScheduleBody {
        message: format!("Anthropic beta headers reload scheduled for every {hours} hours"),
        status: "success",
        interval_hours: hours,
        timestamp: naive_utc_isoformat(now),
    }))
}

pub(super) async fn cancel_beta_headers_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<CancelBody>, ReloadError> {
    require_write_admin(caller)?;
    let store = write_store(&state, CANCEL_FAILED)?;
    store_call(store.delete_beta_config(), CANCEL_FAILED).await?;
    store_call(store.invalidate_beta_config(), CANCEL_FAILED).await?;
    let now = (state.clock)();
    Ok(Json(CancelBody {
        message: "Anthropic beta headers reload schedule cancelled",
        status: "success",
        timestamp: naive_utc_isoformat(now),
    }))
}
