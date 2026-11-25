use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_models::key_digest::hashing::hash_token;
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::AuthContext;
use crate::cache_activity_routes::date::parse_date;
use crate::spend::reports::scope::ScopeColumn;
use crate::tracing_routes::router::unauthenticated;

use super::error::GlobalReportError;
use super::query::{GlobalReportQuery, GroupBy, decode_global_report_query};
use super::rows::GlobalReport;
use super::store::{GlobalReportHandle, GlobalReportStore, ReportBranch, ReportWindow};

pub(super) async fn global_report_respond(
    State(handle): State<GlobalReportHandle>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if identity.is_none() {
        return unauthenticated();
    }
    let raw = raw_query.unwrap_or_default();
    match global_report(handle.as_deref(), raw.as_bytes()).await {
        Ok(report) => Json(report).into_response(),
        Err(error) => error.into_response(),
    }
}

// LiteLLM validates the query, then reads the dates outside its try, and only then checks the database
async fn global_report(
    store: Option<&dyn GlobalReportStore>,
    raw: &[u8],
) -> Result<GlobalReport, GlobalReportError> {
    let query = decode_global_report_query(raw)?;
    let (Some(start_text), Some(end_text)) =
        (query.start_date.as_deref(), query.end_date.as_deref())
    else {
        return Err(GlobalReportError::MissingDates);
    };
    let start = parse_date(start_text).ok_or(GlobalReportError::DateFormat)?;
    let end = parse_date(end_text).ok_or(GlobalReportError::DateFormat)?;
    let reports = store.ok_or(GlobalReportError::DbNotConnected)?;
    let hashed_key = query.api_key.as_ref().map(hashed_api_key);
    let branch = report_branch(&query, hashed_key.as_deref());
    let window = ReportWindow {
        start: &start,
        end: &end,
    };
    reports
        .report(branch, window)
        .await
        .map_err(GlobalReportError::Query)
}

// the legacy order: a present api_key, then a present internal_user_id, then team_id with customer_id, then group_by
fn report_branch<'query>(
    query: &'query GlobalReportQuery,
    hashed_key: Option<&'query str>,
) -> ReportBranch<'query> {
    if let Some(value) = hashed_key {
        return ReportBranch::Scoped {
            column: ScopeColumn::ApiKey,
            value,
        };
    }
    if let Some(value) = query.internal_user_id.as_deref() {
        return ReportBranch::Scoped {
            column: ScopeColumn::User,
            value,
        };
    }
    if let (Some(team_id), Some(customer_id)) =
        (query.team_id.as_deref(), query.customer_id.as_deref())
    {
        return ReportBranch::TeamCustomer {
            team_id,
            customer_id,
        };
    }
    match query.group_by {
        GroupBy::Team => ReportBranch::Teams(query.team_id.as_deref()),
        GroupBy::Customer => ReportBranch::Customers,
        GroupBy::ApiKey => ReportBranch::Keys,
    }
}

// an sk- api_key is hashed; any other value is the key's hash, which the caller passes as documented
fn hashed_api_key(key: &SecretString) -> String {
    let exposed = key.expose_secret();
    if exposed.starts_with("sk-") {
        return hash_token(exposed);
    }
    exposed.to_owned()
}
