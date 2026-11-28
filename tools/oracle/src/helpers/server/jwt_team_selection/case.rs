use std::collections::BTreeMap;

use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_team_selection::settings::SelectionSettings;
use betterlite_auth::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    Agent,
    Candidates,
    Header,
    Object,
    SpecificTeam,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scenario {
    pub(super) function: Function,
    #[serde(default)]
    pub(super) token: Option<Value>,
    #[serde(default)]
    pub(super) fields: Fields,
    #[serde(default)]
    pub(super) teams: BTreeMap<String, JsonData>,
    #[serde(default)]
    pub(super) aliases: BTreeMap<String, Vec<JsonData>>,
    #[serde(default)]
    pub(super) agents: Vec<Value>,
    #[serde(default)]
    pub(super) allowed_team_ids: Vec<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) requested_model: Option<String>,
    #[serde(default)]
    pub(super) route: String,
    #[serde(default)]
    pub(super) request_method: Option<String>,
    // an absent switch keeps LiteLLM_JWTAuth's false default; Option<bool> keeps the bool count down
    #[serde(default)]
    pub(super) registered_route: Option<bool>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) enforce_rbac: Option<bool>,
    #[serde(default)]
    pub(super) is_proxy_admin: Option<bool>,
    #[serde(default)]
    pub(super) database: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Fields {
    pub(super) team_id_jwt_field: Option<String>,
    pub(super) team_ids_jwt_field: Option<String>,
    pub(super) team_id_default: Option<String>,
    pub(super) team_alias_jwt_field: Option<String>,
    pub(super) agent_id_jwt_field: Option<String>,
    pub(super) team_id_upsert: Option<bool>,
    pub(super) team_claim_fallback: Option<bool>,
    pub(super) fallback_to_db_teams: Option<bool>,
    #[serde(rename = "enforce_team_based_model_access")]
    pub(super) team_model_access: Option<bool>,
    // absent keeps the pinned default list, as LiteLLM_JWTAuth's own default does
    pub(super) team_allowed_routes: Option<Vec<String>>,
}

pub(super) fn claim_settings(fields: &Fields) -> JwtAuthSettings {
    let mut settings = JwtAuthSettings::default();
    settings.team_id_jwt_field = fields.team_id_jwt_field.clone();
    settings.team_ids_jwt_field = fields.team_ids_jwt_field.clone();
    settings.team_id_default = fields.team_id_default.clone();
    settings.team_alias_jwt_field = fields.team_alias_jwt_field.clone();
    settings.agent_id_jwt_field = fields.agent_id_jwt_field.clone();
    settings
}

pub(super) fn selection_settings(fields: &Fields) -> SelectionSettings {
    let mut settings = SelectionSettings::default();
    settings.teams.team_id_jwt_field = fields.team_id_jwt_field.clone();
    settings.teams.team_ids_jwt_field = fields.team_ids_jwt_field.clone();
    settings.teams.team_id_default = fields.team_id_default.clone();
    settings.teams.team_alias_jwt_field = fields.team_alias_jwt_field.clone();
    settings.teams.team_id_upsert = fields.team_id_upsert.unwrap_or(false);
    settings.routing.team_claim_fallback = fields.team_claim_fallback.unwrap_or(false);
    settings.routing.fallback_to_db_teams = fields.fallback_to_db_teams.unwrap_or(false);
    settings.scopes.team_model_access = fields.team_model_access.unwrap_or(false);
    if let Some(routes) = &fields.team_allowed_routes {
        settings.routes.team_allowed_routes.clone_from(routes);
    }
    settings
}

// the case sets one protected route; the fake registry LiteLLM reads accepts the same route and method
pub(super) fn registry(scenario: &Scenario) -> PassThroughRegistry {
    if !scenario.registered_route.unwrap_or(false) {
        return PassThroughRegistry {
            root_path: String::new(),
            registrations: Vec::new(),
        };
    }
    let method = scenario
        .request_method
        .as_deref()
        .unwrap_or_default()
        .to_uppercase();
    PassThroughRegistry {
        root_path: String::new(),
        registrations: vec![(
            format!("p1:exact:{}:{method}", scenario.route),
            PassThroughMetadata {
                endpoint_id: "p1".to_owned(),
                route_type: PassThroughRouteType::Exact,
                path: scenario.route.clone(),
                methods: vec![method],
                auth: Some(true),
            },
        )],
    }
}
