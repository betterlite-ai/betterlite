use std::collections::BTreeSet;

use axum::Json;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::effective_user_role;

use super::LogListState;
use super::authz::{AuthorizationRequest, authorize};
use super::conditions::{BuiltConditions, ConditionInputs, build_conditions};
use super::error::LogListError;
use super::order::order_clause;
use super::page::{PageWindow, bounded_total, count_query, page_query, total_pages};
use super::scope::LogListScope;
use super::store::LogListStore;
use crate::plugin_routes::routes::caller_identity;
use crate::spend::log_payload::metadata::hydrate_log_metadata;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::log_query::validate::{SortColumn, SortDirection, SpendLogChoices};
use crate::spend::log_query::{SpendLogQuery, decode_spend_log_query};
use crate::spend::log_results::projection::{
    SpendLogListRow, SpendLogsPage, build_ui_spend_logs_response,
};
use crate::spend::sessions::grouped::sql::where_clause;
use crate::spend::sessions::grouped::{GroupedSessionRequest, ui_session_grouped_spend_logs};
use crate::spend::visibility::owners::OwnerPair;
use crate::spend::visibility::roles::is_admin_view_safe;
use crate::team_access::caller::TeamCaller;

// the query, its closed values and the authorized conditions, once validation and B1704's checks have run
struct AuthorizedPage<'query> {
    store: &'query dyn LogListStore,
    route: SpendLogRoute,
    query: &'query SpendLogQuery,
    choices: &'query SpendLogChoices,
    caller: &'query TeamCaller,
    built: BuiltConditions,
}

// ui_view_spend_logs: framework validation, the missing database, the closed choice values, then the authorized page read
pub(super) async fn execute(
    state: &LogListState,
    route: SpendLogRoute,
    principal: &Principal,
    raw_query: Option<&str>,
) -> Result<Response, LogListError> {
    let query = decode_spend_log_query(raw_query);
    if let Some(issues) = query.framework_issues() {
        return Err(LogListError::Validation(issues));
    }
    let store = state.store.as_deref().ok_or(LogListError::NoDatabase)?;
    let choices = query.choices()?;
    let prepared = query.prepare(route)?;
    let caller = team_caller(principal)?;
    let authorization = authorize(&AuthorizationRequest {
        caller: &caller,
        query: &query,
        route,
        scope: state.scope.as_deref(),
    })
    .await?;
    let built = build_conditions(&ConditionInputs {
        query: &query,
        prepared: &prepared,
        choices: &choices,
        authorization: &authorization,
    });
    let page = AuthorizedPage {
        store,
        route,
        query: &query,
        choices: &choices,
        caller: &caller,
        built,
    };
    read_page(state, page).await
}

// the UserAPIKeyAuth identity the route authorizes with, from the caller's key or its user record
fn team_caller(principal: &Principal) -> Result<TeamCaller, LogListError> {
    let identity = caller_identity(principal).map_err(LogListError::Identity)?;
    Ok(TeamCaller {
        user_id: identity.user_id,
        user_role: Some(effective_user_role(identity.user_role.as_deref())),
        team_id: None,
        team_service_account: false,
    })
}

// a grouped UI page belongs to B1731's keyset pager, and every other request to the offset read below
async fn read_page(
    state: &LogListState,
    page: AuthorizedPage<'_>,
) -> Result<Response, LogListError> {
    if is_delegated(page.route, page.query, page.choices) {
        return grouped_page(state, page).await;
    }
    flat_page(state, page).await
}

// _ui_session_grouped_spend_logs: the conditions and their values, the requested page, the direction and any keyset cursor
async fn grouped_page(
    state: &LogListState,
    page: AuthorizedPage<'_>,
) -> Result<Response, LogListError> {
    let sessions = state
        .sessions
        .as_deref()
        .ok_or(LogListError::GroupedStore)?;
    let built = page.built;
    let request = GroupedSessionRequest {
        conditions: &built.conditions,
        params: built.params,
        page: page.query.page.page,
        page_size: page.query.page.page_size,
        descending: page.choices.sort_direction == SortDirection::Descending,
        session_cursor: page.query.page.session_cursor.as_deref(),
    };
    let grouped =
        ui_session_grouped_spend_logs(sessions, state.enricher.as_deref(), &request).await?;
    Ok(Json(grouped).into_response())
}

// the bounded count, the page read and its enrichment; the metadata hydration follows the fetched-owner recheck
async fn flat_page(
    state: &LogListState,
    page: AuthorizedPage<'_>,
) -> Result<Response, LogListError> {
    let grouped = page.query.flags.group_by_session && page.query.filters.search.is_none();
    let joined = where_clause(&page.built.conditions);
    let count = count_query(&joined, grouped, &page.built.params);
    let raw_total = page.store.count_matches(&count).await?;
    let order_sql = order_clause(page.choices, page.built.request_id_index, grouped);
    let page_number = page.query.page.page;
    let page_size = page.query.page.page_size;
    let window = PageWindow {
        page_size,
        offset: page_number.saturating_sub(1).saturating_mul(page_size),
    };
    let rows_query = page_query(&joined, &order_sql, grouped, &page.built.params, window);
    let mut rows = page.store.list_rows(&rows_query).await?;
    let scope = state.scope.as_deref();
    recheck_fetched_owners(scope, page.caller, page.route, page.query, &rows).await?;
    for row in &mut rows {
        if let Some(metadata) = row.metadata.as_mut() {
            hydrate_log_metadata(metadata);
        }
    }
    let (total, total_is_capped) = bounded_total(raw_total);
    let result = SpendLogsPage {
        data: rows,
        total,
        page: page.query.page.page,
        page_size: page.query.page.page_size,
        total_pages: total_pages(total, page.query.page.page_size),
        total_is_capped,
    };
    let enrich = !matches!(page.route, SpendLogRoute::PublicV2);
    let enriched = build_ui_spend_logs_response(state.enricher.as_deref(), result, enrich).await?;
    Ok(Json(enriched).into_response())
}

// B1731 owns the keyset pager: it runs only for the UI route, one session per row, no id or search lookup, sorted by start time
fn is_delegated(route: SpendLogRoute, query: &SpendLogQuery, choices: &SpendLogChoices) -> bool {
    query.flags.group_by_session
        && matches!(route, SpendLogRoute::Ui)
        && query.filters.request_id.is_none()
        && query.filters.search.is_none()
        && choices.sort_column == SortColumn::StartTime
}

// _assert_user_owns_fetched_spend_rows: the precheck and the page read see the table at different moments, so every fetched owner pair is checked again
async fn recheck_fetched_owners(
    scope: Option<&dyn LogListScope>,
    caller: &TeamCaller,
    route: SpendLogRoute,
    query: &SpendLogQuery,
    rows: &[SpendLogListRow],
) -> Result<(), LogListError> {
    let Some(request_id) = query.filters.request_id.clone() else {
        return Ok(());
    };
    if !matches!(route, SpendLogRoute::Ui) || is_admin_view_safe(caller) {
        return Ok(());
    }
    let owners: BTreeSet<OwnerPair> = rows
        .iter()
        .map(|row| OwnerPair {
            user: row.user.clone(),
            team_id: row.team_id.clone(),
        })
        .collect();
    let Some(visibility) = scope else {
        return Err(LogListError::ForbiddenRequestId { request_id });
    };
    for owner in &owners {
        if !visibility.can_view_row_owner(caller, owner).await? {
            return Err(LogListError::ForbiddenRequestId { request_id });
        }
    }
    Ok(())
}
