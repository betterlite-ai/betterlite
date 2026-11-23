use axum::Json;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::effective_user_role;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Serialize;

use super::LogPayloadState;
use super::content::{RequestResponsePayload, resolve_request_response_payload};
use super::dates::{DetailDates, decode_detail_dates};
use super::error::LogPayloadError;
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;
use crate::spend::visibility::roles::is_admin_view_safe;
use crate::spend::visibility::service::SpendLogVisibility;
use crate::team_access::caller::TeamCaller;
use crate::tracing_routes::router::unauthenticated;

#[derive(Serialize)]
#[serde(untagged)]
enum PayloadBody {
    Callback(JsonData),
    Row(RequestResponsePayload),
}

// ui_view_request_response_for_request_id
pub(super) async fn view_request_payload(
    State(state): State<LogPayloadState>,
    identity: Option<Extension<AuthContext>>,
    Path(request_id): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    match view(
        &state,
        &context.principal,
        &request_id,
        raw_query.as_deref(),
    )
    .await
    {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

// a non-admin is authorized before the dates are read or any callback is asked
async fn view(
    state: &LogPayloadState,
    principal: &Principal,
    request_id: &str,
    raw_query: Option<&str>,
) -> Result<Option<PayloadBody>, LogPayloadError> {
    let identity = caller_identity(principal).map_err(LogPayloadError::Caller)?;
    let caller = TeamCaller {
        user_id: identity.user_id,
        user_role: Some(effective_user_role(identity.user_role.as_deref())),
        team_id: None,
        team_service_account: false,
    };
    let admin = is_admin_view_safe(&caller);
    let visibility = state.visibility.as_deref();
    if !admin {
        let checks = visibility.ok_or(LogPayloadError::NoDatabase)?;
        checks
            .assert_can_view_request_id(&caller, request_id)
            .await?;
    }
    let (start_date, end_date) = detail_dates(raw_query);
    let window = decode_detail_dates(start_date.as_deref(), end_date.as_deref())?;
    let resolved = match visibility {
        Some(checks) => Some(checks.resolve_payload_row(&caller, request_id).await?),
        None => None,
    };
    let storage_key = resolved
        .as_ref()
        .map_or(request_id, |found| found.storage_key.as_str());
    let owner_check = if admin {
        None
    } else {
        visibility.map(|checks| (checks, &caller))
    };
    if let Some(payload) =
        callback_payload(state, storage_key, &window, owner_check, request_id).await?
    {
        return Ok(Some(PayloadBody::Callback(payload)));
    }
    let Some(row) = resolved.and_then(|found| found.row) else {
        return Ok(None);
    };
    let payload = resolve_request_response_payload(&row, &state.cold_storage).await;
    Ok(Some(PayloadBody::Row(payload)))
}

// the first callback that has the request wins; a non-admin must own what it returns
async fn callback_payload(
    state: &LogPayloadState,
    storage_key: &str,
    window: &DetailDates,
    owner_check: Option<(&SpendLogVisibility, &TeamCaller)>,
    request_id: &str,
) -> Result<Option<JsonData>, LogPayloadError> {
    for logger in state.loggers.payload_loggers() {
        let found = logger
            .request_response_payload(storage_key, window)
            .await
            .map_err(LogPayloadError::Callback)?;
        let Some(fields) = found else {
            continue;
        };
        let payload = JsonData::Object(fields);
        if let Some((checks, caller)) = owner_check {
            checks
                .assert_owns_cold_payload(caller, &payload, request_id)
                .await?;
        }
        return Ok(Some(payload));
    }
    Ok(None)
}

// FastAPI reads each query value once, the last one sent winning, with no trimming
fn detail_dates(raw_query: Option<&str>) -> (Option<String>, Option<String>) {
    let mut start_date = None;
    let mut end_date = None;
    for (name, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "start_date" => start_date = Some(value.into_owned()),
            "end_date" => end_date = Some(value.into_owned()),
            _ => {}
        }
    }
    (start_date, end_date)
}
