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
use super::scope::{ScopeCaller, ScopeColumn, resolve_scope};
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

// the requested scope of a key, user or team route and the caller value it is checked against
#[derive(Clone, Copy)]
struct RouteScope<'request> {
    requested: Option<&'request str>,
    caller_value: ScopeCaller<'request>,
    column: ScopeColumn,
    scope_name: &'static str,
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
    match spend_report_dates(query.start_date.as_deref(), query.end_date.as_deref()) {
        Ok(dates) => dispatch_report(store, &caller, &query, &dates, route).await,
        Err(error) => {
            let detail = error.to_string();
            (
                StatusCode::BAD_REQUEST,
                Json(TextDetail { detail: &detail }),
            )
                .into_response()
        }
    }
}

// the report a key, user, team or organization route answers over the caller and its checked dates
async fn dispatch_report(
    store: &dyn ScopedReportStore,
    caller: &ReportCaller,
    query: &ScopedReportQuery,
    dates: &SpendReportDates,
    route: ReportRoute,
) -> Response {
    let hashed_key = query.api_key.as_deref().map(requested_key);
    let request = match route {
        ReportRoute::Key => RouteScope {
            requested: hashed_key.as_deref(),
            caller_value: ScopeCaller::Key(caller.api_key.as_ref()),
            column: ScopeColumn::ApiKey,
            scope_name: "api_key",
        },
        ReportRoute::User => RouteScope {
            requested: query.internal_user_id.as_deref(),
            caller_value: ScopeCaller::Value(caller.org_caller.user_id.as_deref()),
            column: ScopeColumn::User,
            scope_name: "internal_user_id",
        },
        ReportRoute::Team => RouteScope {
            requested: query.team_id.as_deref(),
            caller_value: ScopeCaller::Value(caller.team_id.as_deref()),
            column: ScopeColumn::TeamId,
            scope_name: "team_id",
        },
        ReportRoute::Organization => {
            return organization_report(store, caller, query.organization_id.as_deref(), dates)
                .await;
        }
    };
    let scope = match resolve_scope(
        request.requested,
        request.caller_value,
        caller.org_caller.admin_view,
        request.scope_name,
    ) {
        Ok(scope) => scope,
        Err(error) => return error.respond(),
    };
    scoped_report_response(store, dates, request.column, &scope).await
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

// the scoped report query over one resolved scope and its window, answered as JSON rows
async fn scoped_report_response(
    store: &dyn ScopedReportStore,
    window: &SpendReportDates,
    column: ScopeColumn,
    scope: &str,
) -> Response {
    report_rows(
        store
            .scoped_report(ScopedReportRequest {
                column,
                start: &window.start,
                end: &window.end,
                value: scope,
            })
            .await,
    )
}

async fn organization_report(
    store: &dyn ScopedReportStore,
    caller: &ReportCaller,
    requested: Option<&str>,
    window: &SpendReportDates,
) -> Response {
    let Some(organization_id) = requested
        .filter(|value| !value.is_empty())
        .or(caller.org_id.as_deref())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(TextDetail {
                detail: NO_ORGANIZATION_DETAIL,
            }),
        )
            .into_response();
    };
    let lookup = StoreOrgLookup { store };
    if let Err(error) = verify_org_access(organization_id, &caller.org_caller, &lookup).await {
        return error.into_response();
    }
    let team_ids = match store.organization_team_ids(organization_id).await {
        Ok(team_ids) => team_ids,
        Err(error) => {
            tracing::warn!(error = %error, "the organization teams could not be read");
            return internal_error();
        }
    };
    report_rows(
        store
            .organization_report(OrganizationReportRequest {
                start: &window.start,
                end: &window.end,
                organization_id,
                team_ids: &team_ids,
            })
            .await,
    )
}
