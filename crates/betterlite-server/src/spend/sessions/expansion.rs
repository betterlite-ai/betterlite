use async_trait::async_trait;
use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::effective_user_role;
use serde::Serialize;

use super::SessionsState;
use super::error::SessionsError;
use super::params::SessionParam;
use super::ports::SessionsStore;
use super::query::{SessionExpansionQuery, decode_session_query};
use super::rows::{SessionSpendRow, session_row_count_query, session_rows_query};
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;
use crate::spend::log_payload::metadata::hydrate_log_metadata;
use crate::spend::visibility::roles::{can_user_view_spend_log, is_admin_view_safe};
use crate::spend::visibility::service::SpendLogVisibility;
use crate::team_access::caller::TeamCaller;
use crate::tracing_routes::router::unauthenticated;

// _get_permitted_team_ids_for_spend_logs_or_empty behind a port, so a failed lookup narrows the caller to their own rows instead of losing the scope entirely
#[async_trait]
pub trait SessionTeamScope: Send + Sync {
    async fn team_scope(&self, caller: &TeamCaller) -> Vec<String>;
}

#[async_trait]
impl SessionTeamScope for SpendLogVisibility {
    // No test builds B1704's service, whose user lookup needs a database and SSO; a mutant of this delegation never runs.
    #[cfg_attr(test, mutants::skip)]
    async fn team_scope(&self, caller: &TeamCaller) -> Vec<String> {
        self.permitted_teams_or_empty(caller).await
    }
}

#[derive(Serialize)]
struct SessionExpansionPage {
    data: Vec<SessionSpendRow>,
    total: i64,
    page: i64,
    page_size: i64,
    total_pages: i64,
}

// ui_view_session_spend_logs
pub(super) async fn ui_view_session_spend_logs(
    State(state): State<SessionsState>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    match view(&state, &context.principal, raw_query.as_deref()).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn view(
    state: &SessionsState,
    principal: &Principal,
    raw_query: Option<&str>,
) -> Result<SessionExpansionPage, SessionsError> {
    let identity = caller_identity(principal).map_err(SessionsError::Identity)?;
    let query = decode_session_query(raw_query).map_err(SessionsError::Validation)?;
    let store = state.store.as_deref().ok_or(SessionsError::NoDatabase)?;
    let caller = TeamCaller {
        user_id: identity.user_id,
        user_role: Some(effective_user_role(identity.user_role.as_deref())),
        team_id: None,
        team_service_account: false,
    };
    expansion_page(store, state, &caller, &query).await
}

// the count is uncapped and runs before the page's rows, and only metadata is hydrated
async fn expansion_page(
    store: &dyn SessionsStore,
    state: &SessionsState,
    caller: &TeamCaller,
    query: &SessionExpansionQuery,
) -> Result<SessionExpansionPage, SessionsError> {
    let (where_clause, params) = session_scope(state, caller, query.session_id.clone()).await;
    let total = store
        .session_row_count(&session_row_count_query(&where_clause, params.clone()))
        .await?;
    let offset = query.page.saturating_sub(1).saturating_mul(query.page_size);
    let mut data = store
        .session_rows(&session_rows_query(
            &where_clause,
            params,
            query.page_size,
            offset,
        ))
        .await?;
    for row in &mut data {
        if let Some(metadata) = row.metadata.as_mut() {
            hydrate_log_metadata(metadata);
        }
    }
    Ok(SessionExpansionPage {
        data,
        total,
        page: query.page,
        page_size: query.page_size,
        total_pages: total.div_ceil(query.page_size),
    })
}

// the WHERE clause the expansion reads run: the exact session id, then the admin's whole table or a non-admin's own rows and permitted teams, with a null user id still bound
async fn session_scope(
    state: &SessionsState,
    caller: &TeamCaller,
    session_id: String,
) -> (String, Vec<SessionParam>) {
    let mut conditions = vec![String::from("session_id = $1")];
    let mut params = vec![SessionParam::Text(session_id)];
    if is_admin_view_safe(caller) {
        return (conditions.join(" AND "), params);
    }
    let team_ids = if can_user_view_spend_log(caller) {
        permitted_teams(state, caller).await
    } else {
        Vec::new()
    };
    let user_index = params.len().saturating_add(1);
    params.push(SessionParam::MaybeText(caller.user_id.clone()));
    if team_ids.is_empty() {
        conditions.push(format!("\"user\" = ${user_index}"));
    } else {
        let teams_index = user_index.saturating_add(1);
        params.push(SessionParam::Texts(team_ids));
        conditions.push(format!(
            "(\"user\" = ${user_index} OR team_id = ANY(${teams_index}::text[]))"
        ));
    }
    (conditions.join(" AND "), params)
}

// a proxy without the permitted-team lookup scopes every caller to their own rows
async fn permitted_teams(state: &SessionsState, caller: &TeamCaller) -> Vec<String> {
    let Some(scope) = state.scope.as_deref() else {
        return Vec::new();
    };
    scope.team_scope(caller).await
}
