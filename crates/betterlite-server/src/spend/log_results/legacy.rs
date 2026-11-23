use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::effective_user_role;
use indexmap::IndexMap;

use super::LogResultsState;
use super::capped::{CappedRows, find_spend_logs};
use super::error::LogResultsError;
use super::filters::{LegacyFilters, scoped_user_id};
use super::query::decode_legacy_spend_query;
use super::rows::spend_log_rows_query;
use super::summary::pad_summary;
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;
use crate::spend::log_payload::daily_summary::{
    SummaryFilters, SummaryValue, daily_summary_items, daily_summary_sql,
};
use crate::tracing_routes::router::unauthenticated;

enum SpendLogsReply {
    Rows(CappedRows),
    Summary(Vec<IndexMap<String, SummaryValue>>),
}

impl IntoResponse for SpendLogsReply {
    fn into_response(self) -> Response {
        match self {
            Self::Rows(rows) => rows.into_response(),
            Self::Summary(items) => Json(items).into_response(),
        }
    }
}

// view_spend_logs
pub(super) async fn view_spend_logs(
    State(state): State<LogResultsState>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    match logs_reply(&state, &context.principal, raw_query.as_deref()).await {
        Ok(reply) => reply.into_response(),
        Err(error) => error.into_response(),
    }
}

// the two dates together pick the summary or the filtered rows; one date alone always lists rows
async fn logs_reply(
    state: &LogResultsState,
    principal: &Principal,
    raw_query: Option<&str>,
) -> Result<SpendLogsReply, LogResultsError> {
    let identity = caller_identity(principal).map_err(LogResultsError::Identity)?;
    let query = decode_legacy_spend_query(raw_query.unwrap_or_default().as_bytes())?;
    let user_id = scoped_user_id(
        effective_user_role(identity.user_role.as_deref()),
        identity.user_id.as_deref(),
        query.user_id.as_deref(),
    );
    let store = state.store.as_deref().ok_or(LogResultsError::NoDatabase)?;
    let filters = LegacyFilters::resolve(&query, user_id)?;
    let Some(window) = filters.window.as_ref().filter(|_window| query.summarize) else {
        let rows = spend_log_rows_query(&filters);
        return Ok(SpendLogsReply::Rows(find_spend_logs(store, &rows).await?));
    };
    let summary_query = daily_summary_sql(
        &window.start_iso,
        &window.end_iso,
        SummaryFilters {
            api_key: filters.api_key.as_deref(),
            request_id: filters.request_id.as_deref(),
            user_id: filters.user_id.as_deref(),
        },
    );
    let summary = store.daily_summary(&summary_query).await?;
    let items = daily_summary_items(&summary)?;
    Ok(SpendLogsReply::Summary(pad_summary(items, window.end_date)))
}
