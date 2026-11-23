use axum::Json;
use axum::extract::{Extension, State};
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::beta_config::reload_config_value;
use super::caller::{caller_principal, require_proxy_admin};
use super::error::{ReloadError, WrappedCause};
use super::hours::parse_hours;
use super::wire::naive_utc_isoformat;

const SCHEDULE_FAILED: &str = "Failed to schedule anthropic beta headers reload";
const CANCEL_FAILED: &str = "Failed to cancel anthropic beta headers reload";

#[derive(Serialize)]
struct ScheduleBody {
    message: String,
    status: &'static str,
    interval_hours: i64,
    timestamp: String,
}

#[derive(Serialize)]
struct CancelBody {
    message: &'static str,
    status: &'static str,
    timestamp: String,
}

pub(super) async fn schedule_anthropic_beta_headers_reload(
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
        return Err(ReloadError::wrapped(
            SCHEDULE_FAILED,
            WrappedCause::MissingDatabase,
        ));
    };
    store
        .write_beta_config(reload_config_value(Some(hours), false))
        .await
        .map_err(|source| ReloadError::wrapped(SCHEDULE_FAILED, WrappedCause::Store(source)))?;
    store
        .invalidate_beta_config()
        .await
        .map_err(|source| ReloadError::wrapped(SCHEDULE_FAILED, WrappedCause::Store(source)))?;
    let now = (state.clock)();
    let body = ScheduleBody {
        message: format!("Anthropic beta headers reload scheduled for every {hours} hours"),
        status: "success",
        interval_hours: hours,
        timestamp: naive_utc_isoformat(now),
    };
    Ok(Json(body).into_response())
}

pub(super) async fn cancel_anthropic_beta_headers_reload(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_proxy_admin(&principal)?;
    let Some(store) = state.config.as_deref() else {
        return Err(ReloadError::wrapped(
            CANCEL_FAILED,
            WrappedCause::MissingDatabase,
        ));
    };
    store
        .delete_beta_config()
        .await
        .map_err(|source| ReloadError::wrapped(CANCEL_FAILED, WrappedCause::Store(source)))?;
    store
        .invalidate_beta_config()
        .await
        .map_err(|source| ReloadError::wrapped(CANCEL_FAILED, WrappedCause::Store(source)))?;
    let now = (state.clock)();
    let body = CancelBody {
        message: "Anthropic beta headers reload schedule cancelled",
        status: "success",
        timestamp: naive_utc_isoformat(now),
    };
    Ok(Json(body).into_response())
}
