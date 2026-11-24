use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_models::key_digest::hashing::hash_token;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::organization_create::access::verify_org_access;
use crate::tracing_routes::router::unauthenticated;

use super::caller::ReportCaller;
use super::dates::{SpendReportDates, spend_report_dates};
use super::error::{DB_NOT_CONNECTED_ERROR, TextDetail, internal_error};
use super::query::{ScopedReportQuery, decode_scoped_report_query};
use super::scope::{ScopeColumn, resolve_scope};
use super::store::{
    OrganizationReportRequest, ScopedReportError, ScopedReportHandle, ScopedReportRequest,
    ScopedReportStore, StoreOrgLookup,
};

const NO_ORGANIZATION_DETAIL: &str =
    "No organization_id associated with this API key; pass an organization_id query_param";

#[derive(Clone, Copy, Debug)]
pub(super) enum ReportRoute {
    Key,
    User,
    Team,
    Organization,
}

pub(super) async fn report_respond(
    State(handle): State<ScopedReportHandle>,
    Extension(route): Extension<ReportRoute>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    let Some(store) = handle.as_deref() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(TextDetail {
                detail: DB_NOT_CONNECTED_ERROR,
            }),
        )
            .into_response();
    };
    let caller = match ReportCaller::from_principal(&context.principal) {
        Ok(caller) => caller,
        Err(error) => {
            tracing::warn!(error = %error, "the caller facts could not be read");
            return internal_error();
        }
    };
    let query = decode_scoped_report_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let dates = match report_dates(&query) {
        Ok(dates) => dates,
        Err(response) => return response,
    };
    match route {
        ReportRoute::Key => {
            let requested = query.api_key.as_deref().map(requested_key);
            scoped_report(
                store,
                &caller,
                &dates,
                requested.as_deref(),
                caller.api_key.as_deref(),
                ScopeColumn::ApiKey,
                "api_key",
            )
            .await
        }
        ReportRoute::User => {
            scoped_report(
                store,
                &caller,
                &dates,
                query.internal_user_id.as_deref(),
                caller.org_caller.user_id.as_deref(),
                ScopeColumn::User,
                "internal_user_id",
            )
            .await
        }
        ReportRoute::Team => {
            scoped_report(
                store,
                &caller,
                &dates,
                query.team_id.as_deref(),
                caller.team_id.as_deref(),
                ScopeColumn::TeamId,
                "team_id",
            )
            .await
        }
        ReportRoute::Organization => organization_report(store, &caller, &query, &dates).await,
    }
}

// _parse_spend_report_date_range: both boundaries are required and a failed parse is this one 400
fn report_dates(query: &ScopedReportQuery) -> Result<SpendReportDates, Response> {
    spend_report_dates(query.start_date.as_deref(), query.end_date.as_deref()).map_err(|error| {
        let detail = error.to_string();
        (
            StatusCode::BAD_REQUEST,
            Json(TextDetail { detail: &detail }),
        )
            .into_response()
    })
}

// the key route hashes an sk- prefixed request parameter before it is compared with the caller's logged key
fn requested_key(value: &str) -> String {
    if value.starts_with("sk-") {
        return hash_token(value);
    }
    value.to_owned()
}

fn report_rows<Row>(result: Result<Vec<Row>, ScopedReportError>) -> Response
where
    Row: Serialize,
{
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "a spend report query failed");
            internal_error()
        }
    }
}

// _resolve_spend_report_scope over one route's requested and caller values
async fn scoped_report(
    store: &dyn ScopedReportStore,
    caller: &ReportCaller,
    dates: &SpendReportDates,
    requested: Option<&str>,
    caller_value: Option<&str>,
    column: ScopeColumn,
    scope_name: &'static str,
) -> Response {
    let scope = match resolve_scope(
        requested,
        caller_value,
        caller.org_caller.admin_view,
        scope_name,
    ) {
        Ok(scope) => scope,
        Err(error) => return error.respond(),
    };
    report_rows(
        store
            .scoped_report(ScopedReportRequest {
                column,
                start: &dates.start,
                end: &dates.end,
                value: &scope,
            })
            .await,
    )
}

async fn organization_report(
    store: &dyn ScopedReportStore,
    caller: &ReportCaller,
    query: &ScopedReportQuery,
    dates: &SpendReportDates,
) -> Response {
    let requested = query
        .organization_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let Some(target) = requested.or(caller.org_id.as_deref()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(TextDetail {
                detail: NO_ORGANIZATION_DETAIL,
            }),
        )
            .into_response();
    };
    if let Err(error) =
        verify_org_access(target, &caller.org_caller, &StoreOrgLookup { store }).await
    {
        return error.into_response();
    }
    let team_ids = match store.organization_team_ids(target).await {
        Ok(team_ids) => team_ids,
        Err(error) => {
            tracing::warn!(error = %error, "the organization teams could not be read");
            return internal_error();
        }
    };
    report_rows(
        store
            .organization_report(OrganizationReportRequest {
                start: &dates.start,
                end: &dates.end,
                organization_id: target,
                team_ids: &team_ids,
            })
            .await,
    )
}
