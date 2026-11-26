use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_context_sources::router_alias::{
    RouterAliasInput, RouterAliasState, resolve_router_alias,
};
use betterlite_auth::auth_context_sources::router_settings::{
    TeamRouterSettings, hierarchical_router_settings,
};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use serde_json::{Value, json};

use super::{Input, LookupMode};

enum TeamAnswer {
    Columns(BTreeMap<String, JsonData>),
    Missing,
    Failed,
}

struct TeamsScript {
    answer: TeamAnswer,
}

#[async_trait]
impl TeamRouterSettings for TeamsScript {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        match &self.answer {
            TeamAnswer::Columns(columns) => Ok(Some(columns.clone())),
            TeamAnswer::Missing => Ok(None),
            TeamAnswer::Failed => Err(TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            }),
        }
    }
}

// The router-alias call carries the key's settings alone, so its team source never answers.
const NO_TEAMS: TeamsScript = TeamsScript {
    answer: TeamAnswer::Missing,
};

fn team_answer(input: &Input) -> TeamAnswer {
    if matches!(input.team_lookup.unwrap_or_default(), LookupMode::Error) {
        return TeamAnswer::Failed;
    }
    match &input.team_settings {
        Some(columns) => TeamAnswer::Columns(columns.clone()),
        None => TeamAnswer::Missing,
    }
}

pub(super) async fn resolved_settings(input: &Input) -> Option<JsonObject> {
    let teams = TeamsScript {
        answer: team_answer(input),
    };
    hierarchical_router_settings(
        input.key_settings.as_ref(),
        input.team_id.as_deref(),
        &teams,
    )
    .await
}

fn router_state(input: &Input) -> RouterAliasState {
    let route = RouteValue::Text(input.route.as_str());
    let routed = is_llm_api_route(&route) && input.router_available;
    match (routed, input.marker_set) {
        (false, already_resolved) => RouterAliasState::Ineligible { already_resolved },
        (true, true) => RouterAliasState::Resolved,
        (true, false) => RouterAliasState::Fresh,
    }
}

pub(super) async fn alias_rewrite(input: &Input) -> Value {
    let alias_input = RouterAliasInput {
        state: router_state(input),
        passthrough: input.passthrough,
        requested_model: input.requested_model.as_deref(),
        raw_body_available: input.raw_body_available,
        key_router_settings: input.settings.as_ref(),
        team_id: input.team_id.as_deref(),
        client_requested_model: input.client_requested_model.as_deref(),
    };
    let rewrite = resolve_router_alias(&alias_input, &NO_TEAMS).await;
    json!({
        "marker_set": rewrite.marker_set,
        "client_requested_model": rewrite.client_requested_model.or_else(|| input.client_requested_model.clone()),
        "target": rewrite.target.or_else(|| input.requested_model.clone()),
    })
}
