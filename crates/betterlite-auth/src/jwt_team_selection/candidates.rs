use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::identity_rows::Payload;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_request_models::RequestedModels;
use crate::auth_route_policy::jwt::allowed_routes_check;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;
use crate::entity_model_access::team::{TeamAccessRequest, can_team_access_model};
use crate::identity_grants::team_grants::team_json_columns;
use crate::pass_through_registry::PassThroughRegistry;
use crate::user_roles::UserRole;
use crate::virtual_key_routes::caller::{
    AllowedRoutes, RouteCaller, check_passthrough_route_access, jwt_team_grant_pass_through,
};
use crate::virtual_key_routes::pass_through::is_protected_pass_through;

use super::error::TeamSelectionError;
use super::settings::SelectionSettings;

// what the candidate scan asks for: the token's teams, the model it wants and the route it is calling
pub struct CandidateRequest<'request> {
    pub team_ids: &'request BTreeSet<String>,
    pub requested_model: Option<&'request str>,
    pub route: &'request str,
    pub request_method: Option<&'request str>,
}

// the reads a candidate makes: the team rows, the registered pass-throughs and the router-backed model view
pub struct CandidateContext<'context> {
    pub lookup: &'context TeamLookup,
    pub registry: &'context PassThroughRegistry,
    pub models: &'context ModelAccessContext<'context>,
}

enum CandidateVerdict {
    // the row did not resolve, so the failure is swallowed
    Absent,
    // the row resolved but grants neither the model nor the route
    Skip,
    PassthroughDenied,
    Selected(LiteLLMTeamTable),
}

// find_team_with_model_access: the first candidate that grants the model and the route
pub async fn find_team_with_model_access(
    request: &CandidateRequest<'_>,
    deps: &CandidateContext<'_>,
    settings: &SelectionSettings,
) -> Result<Option<(String, LiteLLMTeamTable)>, TeamSelectionError> {
    if request.team_ids.is_empty() {
        if settings.scopes.team_model_access && !settings.routing.fallback_to_db_teams {
            return Err(TeamSelectionError::NoTeamsInToken);
        }
        return Ok(None);
    }
    let mut resolved = false;
    let mut passthrough_denied = false;
    for team_id in request.team_ids {
        match candidate_verdict(team_id, request, deps, settings).await {
            CandidateVerdict::Absent => {}
            CandidateVerdict::Skip => resolved = true,
            CandidateVerdict::PassthroughDenied => {
                resolved = true;
                passthrough_denied = true;
            }
            CandidateVerdict::Selected(team) => return Ok(Some((team_id.clone(), team))),
        }
    }
    if passthrough_denied {
        return Err(TeamSelectionError::PassThroughDenied {
            route: request.route.to_owned(),
        });
    }
    if let Some(model) = request.requested_model {
        if resolved || !settings.routing.team_claim_fallback {
            return Err(TeamSelectionError::no_team_for_model(
                model,
                request.team_ids,
            ));
        }
    }
    Ok(None)
}

async fn candidate_verdict(
    team_id: &str,
    request: &CandidateRequest<'_>,
    deps: &CandidateContext<'_>,
    settings: &SelectionSettings,
) -> CandidateVerdict {
    let Ok(team) = get_team_object(
        deps.lookup,
        TeamRequest {
            team_id,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: false,
        },
    )
    .await
    else {
        // LiteLLM continues on every exception here, permission errors included
        return CandidateVerdict::Absent;
    };
    if !grants_model(request.requested_model, &team.team, deps).await {
        return CandidateVerdict::Skip;
    }
    let route_allowed = allowed_routes_check(
        UserRole::Team,
        request.route,
        &[],
        Some(&settings.routes.team_allowed_routes),
    );
    if !route_allowed {
        return CandidateVerdict::Skip;
    }
    if !team_has_passthrough_route_access(
        Some(&team.team),
        request.route,
        request.request_method,
        &settings.routes.team_allowed_routes,
        deps.registry,
    ) {
        return CandidateVerdict::PassthroughDenied;
    }
    CandidateVerdict::Selected(team.team)
}

// team_model_aliases feeds the team grant; an unrequested model passes without reading the grant
async fn grants_model(
    requested_model: Option<&str>,
    team: &LiteLLMTeamTable,
    deps: &CandidateContext<'_>,
) -> bool {
    let Some(model) = requested_model else {
        return true;
    };
    let columns = team_json_columns(team);
    let aliases = columns.model_aliases.unwrap_or_default();
    let key_aliases = BTreeMap::new();
    let grant = TeamAccessRequest {
        team: Some(team),
        team_aliases: &aliases,
        key_aliases: &key_aliases,
        lookup: Some(deps.lookup),
    };
    can_team_access_model(&RequestedModels::One(model.to_owned()), &grant, deps.models)
        .await
        .is_ok()
}

// _team_has_passthrough_route_access: the config grant, then the selected team's own metadata
#[must_use]
pub fn team_has_passthrough_route_access(
    team: Option<&LiteLLMTeamTable>,
    route: &str,
    request_method: Option<&str>,
    team_allowed_routes: &[String],
    registry: &PassThroughRegistry,
) -> bool {
    let method = request_method.map(str::to_uppercase);
    if !is_protected_pass_through(route, method.as_deref(), registry) {
        return true;
    }
    if jwt_team_grant_pass_through(route, team_allowed_routes) {
        return true;
    }
    // JWT team selection is team-scoped, so no key metadata exists: only the team's own grants count
    let metadata = team
        .and_then(|row| row.fields.base.metadata.clone())
        .unwrap_or_default();
    let payload = Payload(metadata);
    let caller = RouteCaller {
        allowed_routes: AllowedRoutes::Unset,
        metadata: None,
        team_metadata: Some(&payload),
        jwt_claims: false,
        has_token: false,
        team_id: None,
    };
    check_passthrough_route_access(route, &caller)
}
