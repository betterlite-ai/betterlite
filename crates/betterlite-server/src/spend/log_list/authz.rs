use super::error::LogListError;
use super::scope::LogListScope;
use crate::spend::log_query::SpendLogQuery;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::visibility::roles::{can_user_view_spend_log, is_admin_view_safe};
use crate::team_access::caller::TeamCaller;

// the authorized narrowing the SQL adds after B1704's checks: nothing, the caller's own rows, or those rows and their teams'
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeClause {
    None,
    OwnUser {
        user_id: Option<String>,
    },
    OwnUserOrTeams {
        user_id: Option<String>,
        team_ids: Vec<String>,
    },
}

pub struct AuthorizationRequest<'request> {
    pub caller: &'request TeamCaller,
    pub query: &'request SpendLogQuery,
    pub route: SpendLogRoute,
    pub scope: Option<&'request dyn LogListScope>,
}

pub struct Authorization {
    pub scope: ScopeClause,
}

// the pinned user/team/ID scope: the id precheck first, then the explicit team filter's permission, then own-user or team expansion
pub async fn authorize(request: &AuthorizationRequest<'_>) -> Result<Authorization, LogListError> {
    let admin = is_admin_view_safe(request.caller);
    let is_request_id_lookup =
        request.query.filters.request_id.is_some() && matches!(request.route, SpendLogRoute::Ui);
    if is_request_id_lookup && !admin {
        precheck_request_id(request).await?;
    }
    if admin {
        return Ok(Authorization {
            scope: ScopeClause::None,
        });
    }
    if let Some(team_id) = request.query.filters.team_id.as_deref() {
        return team_only_scope(request, team_id).await;
    }
    Ok(user_scope(request, is_request_id_lookup).await)
}

// _assert_user_can_view_request_id: without a visible owner pair, including no row at all, the lookup is refused
async fn precheck_request_id(request: &AuthorizationRequest<'_>) -> Result<(), LogListError> {
    let request_id = request
        .query
        .filters
        .request_id
        .as_deref()
        .unwrap_or_default();
    let Some(scope) = request.scope else {
        return Err(LogListError::ForbiddenRequestId {
            request_id: request_id.to_owned(),
        });
    };
    scope
        .assert_can_view_lookup(request.caller, request_id)
        .await?;
    Ok(())
}

// _can_team_member_view_log on an explicit team filter: the team must be visible, and no own-user narrowing follows
async fn team_only_scope(
    request: &AuthorizationRequest<'_>,
    team_id: &str,
) -> Result<Authorization, LogListError> {
    let allowed = match request.scope {
        None => false,
        Some(scope) => scope.can_view_team(request.caller, team_id).await?,
    };
    if !allowed {
        return Err(LogListError::ForbiddenTeam {
            team_id: team_id.to_owned(),
        });
    }
    Ok(Authorization {
        scope: ScopeClause::None,
    })
}

// _get_permitted_team_ids_for_spend_logs_or_empty plus the own-user branch: a null caller id stays a predicate rather than a dropped clause
async fn user_scope(
    request: &AuthorizationRequest<'_>,
    is_request_id_lookup: bool,
) -> Authorization {
    if !(is_request_id_lookup || can_user_view_spend_log(request.caller)) {
        return Authorization {
            scope: ScopeClause::None,
        };
    }
    let team_ids = match request.scope {
        None => Vec::new(),
        Some(scope) => scope.visible_team_ids(request.caller).await,
    };
    let user_id = request.caller.user_id.clone();
    let scope = if team_ids.is_empty() {
        ScopeClause::OwnUser { user_id }
    } else {
        ScopeClause::OwnUserOrTeams { user_id, team_ids }
    };
    Authorization { scope }
}
