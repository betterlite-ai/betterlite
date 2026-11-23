use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::beta_config::interval_hours;
use super::caller::{caller_principal, require_admin_view};
use super::error::{ReloadError, WrappedCause};
use super::wire::{beta_next_run, naive_utc_isoformat};

const STATUS_FAILED: &str = "Failed to get anthropic beta headers reload status";

#[derive(Serialize)]
struct StatusBody {
    scheduled: bool,
    interval_hours: Option<i64>,
    last_run: Option<String>,
    next_run: Option<String>,
}

const NOT_SCHEDULED: StatusBody = StatusBody {
    scheduled: false,
    interval_hours: None,
    last_run: None,
    next_run: None,
};

pub(super) async fn anthropic_beta_headers_status(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_admin_view(&principal)?;
    let Some(store) = state.config.as_deref() else {
        return Ok(Json(NOT_SCHEDULED).into_response());
    };
    let config = store
        .read_beta_config()
        .await
        .map_err(|source| ReloadError::wrapped(STATUS_FAILED, WrappedCause::Store(source)))?;
    let Some(interval) = config.and_then(interval_hours) else {
        return Ok(Json(NOT_SCHEDULED).into_response());
    };
    let now = (state.clock)();
    let last_reload = state.reloads.last_beta_headers_reload();
    let next_run = last_reload.and_then(|last| beta_next_run(last, interval, now));
    let body = StatusBody {
        scheduled: true,
        interval_hours: Some(interval),
        last_run: last_reload.map(naive_utc_isoformat),
        next_run: next_run.map(naive_utc_isoformat),
    };
    Ok(Json(body).into_response())
}
