use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_context_sources::budgets::model_names_for_budget_checks;
use betterlite_auth::auth_context_sources::claude::{
    AliasMap, ClaudeNameSources, RouterNames, claude_code_requested_group,
};
use betterlite_auth::auth_context_sources::router_alias::{
    RouterAliasInput, RouterAliasRewrite, RouterAliasState,
    resolve_router_settings_model_group_alias,
};
use betterlite_auth::auth_context_sources::router_settings::{
    TeamRouterSettings, hierarchical_router_settings,
};
use betterlite_auth::auth_context_sources::sso::{
    SsoConfiguration, SsoRuntime, abbreviate_api_key, has_user_setup_sso,
    is_sso_provider_fully_configured,
};
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    #[default]
    Sso,
    Abbreviate,
    BudgetNames,
    Claude,
    RouterSettings,
    RouterAlias,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouteClass {
    #[default]
    Other,
    LlmApi,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouterState {
    #[default]
    Absent,
    Available,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MarkerState {
    #[default]
    Fresh,
    Resolved,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DispatchState {
    #[default]
    Direct,
    Passthrough,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawBodyState {
    #[default]
    Unavailable,
    Available,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LookupMode {
    #[default]
    Present,
    Absent,
    Error,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Input {
    function: Function,
    scenarios: Vec<SsoScenario>,
    api_key: Option<String>,
    minimum_custom_key_length: Option<i64>,
    model: Option<BudgetModel>,
    requested: Option<String>,
    global_aliases: BTreeMap<String, String>,
    key_aliases: Option<Json>,
    team_model_aliases: Option<Json>,
    hierarchical_aliases: Option<Json>,
    router_model_group_alias: Vec<String>,
    router_model_ids: Vec<String>,
    candidate_ids: BTreeMap<String, Vec<String>>,
    team_id: Option<String>,
    key_settings: Option<Json>,
    team_settings: Option<Json>,
    team_lookup: Option<LookupMode>,
    route_class: Option<RouteClass>,
    router: Option<RouterState>,
    marker: Option<MarkerState>,
    dispatch: Option<DispatchState>,
    requested_model: Option<String>,
    raw_body: Option<RawBodyState>,
    settings: Option<Json>,
    client_requested_model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SsoScenario {
    microsoft_client_id: Option<String>,
    microsoft_client_secret: Option<String>,
    microsoft_tenant: Option<String>,
    google_client_id: Option<String>,
    google_client_secret: Option<String>,
    generic_client_id: Option<String>,
    generic_client_secret: Option<String>,
    generic_authorization_endpoint: Option<String>,
    generic_token_endpoint: Option<String>,
    generic_userinfo_endpoint: Option<String>,
    saml_idp_metadata_url: Option<String>,
    saml_idp_metadata_xml: Option<String>,
    saml_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BudgetModel {
    One(String),
    Many(Vec<String>),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Sso => Ok(sso_answer(input)),
            Function::Abbreviate => Ok(abbreviate_answer(input)),
            Function::BudgetNames => Ok(budget_answer(input)),
            Function::Claude => Ok(claude_answer(input)),
            Function::RouterSettings => {
                let runtime = Builder::new_current_thread().enable_all().build()?;
                Ok(runtime.block_on(router_settings_answer(input)))
            }
            Function::RouterAlias => {
                let runtime = Builder::new_current_thread().enable_all().build()?;
                Ok(runtime.block_on(router_alias_answer(input)))
            }
        }
    }
}

fn sso_answer(input: &Input) -> Value {
    let answers: Vec<Value> = input.scenarios.iter().map(sso_scenario).collect();
    json!(answers)
}

fn sso_scenario(scenario: &SsoScenario) -> Value {
    let configuration = SsoConfiguration {
        microsoft_client_id: scenario.microsoft_client_id.clone(),
        google_client_id: scenario.google_client_id.clone(),
        generic_client_id: scenario.generic_client_id.clone(),
        saml_idp_metadata_url: scenario.saml_idp_metadata_url.clone(),
        saml_idp_metadata_xml: scenario.saml_idp_metadata_xml.clone(),
        google_client_secret: scenario
            .google_client_secret
            .clone()
            .map(SecretString::from),
        microsoft_client_secret: scenario
            .microsoft_client_secret
            .clone()
            .map(SecretString::from),
        microsoft_tenant: scenario.microsoft_tenant.clone(),
        generic_client_secret: scenario
            .generic_client_secret
            .clone()
            .map(SecretString::from),
        generic_authorization_endpoint: scenario.generic_authorization_endpoint.clone(),
        generic_token_endpoint: scenario.generic_token_endpoint.clone(),
        generic_userinfo_endpoint: scenario.generic_userinfo_endpoint.clone(),
    };
    let runtime = SsoRuntime {
        saml_available: scenario.saml_available,
    };
    json!({
        "discovery": has_user_setup_sso(&configuration),
        "ready": is_sso_provider_fully_configured(&configuration, runtime),
    })
}

fn abbreviate_answer(input: &Input) -> Value {
    let api_key = SecretString::from(input.api_key.clone().unwrap_or_default());
    json!(abbreviate_api_key(
        &api_key,
        input.minimum_custom_key_length.unwrap_or_default()
    ))
}

fn budget_answer(input: &Input) -> Value {
    let requested = match &input.model {
        None => RequestedModels::None,
        Some(BudgetModel::One(name)) => RequestedModels::One(name.clone()),
        Some(BudgetModel::Many(names)) => RequestedModels::Many(names.clone()),
    };
    json!(model_names_for_budget_checks(&requested))
}

struct NamesFixture {
    alias_names: Vec<String>,
    model_ids: Vec<String>,
    candidate_ids: BTreeMap<String, Vec<String>>,
}

impl RouterNames for NamesFixture {
    fn model_group_alias_contains(&self, name: &str) -> bool {
        self.alias_names.iter().any(|alias| alias == name)
    }

    fn has_model_id(&self, name: &str) -> bool {
        self.model_ids.iter().any(|id| id == name)
    }

    fn candidate_model_ids_for_route(&self, name: &str, _team_id: Option<&str>) -> Vec<String> {
        self.candidate_ids.get(name).cloned().unwrap_or_default()
    }
}

fn claude_answer(input: &Input) -> Value {
    let router = NamesFixture {
        alias_names: input.router_model_group_alias.clone(),
        model_ids: input.router_model_ids.clone(),
        candidate_ids: input.candidate_ids.clone(),
    };
    let names = ClaudeNameSources {
        global_aliases: &input.global_aliases,
        key_aliases: input.key_aliases.as_ref().map(AliasMap::Json),
        team_model_aliases: input.team_model_aliases.as_ref().map(AliasMap::Json),
        hierarchical_model_group_alias: input.hierarchical_aliases.as_ref().map(AliasMap::Json),
        router: Some(&router),
        team_id: input.team_id.as_deref(),
    };
    let requested = input.requested.as_deref().unwrap_or_default();
    json!(claude_code_requested_group(requested, &names))
}

enum TeamAnswer {
    Present(BTreeMap<String, JsonData>),
    Absent,
    Error,
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
            TeamAnswer::Present(columns) => Ok(Some(columns.clone())),
            TeamAnswer::Absent => Ok(None),
            TeamAnswer::Error => Err(TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            }),
        }
    }
}

struct NoTeams;

#[async_trait]
impl TeamRouterSettings for NoTeams {
    async fn router_settings(
        &self,
        _team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        Ok(None)
    }
}

async fn router_settings_answer(input: &Input) -> Value {
    let teams = TeamsScript {
        answer: team_answer(input),
    };
    let settings = hierarchical_router_settings(
        input.key_settings.as_ref(),
        input.team_id.as_deref(),
        &teams,
    )
    .await;
    json!(settings)
}

fn team_answer(input: &Input) -> TeamAnswer {
    match input.team_lookup.unwrap_or_default() {
        LookupMode::Error => TeamAnswer::Error,
        LookupMode::Absent => TeamAnswer::Absent,
        LookupMode::Present => TeamAnswer::Present(
            input
                .team_settings
                .as_ref()
                .map(columns)
                .unwrap_or_default(),
        ),
    }
}

fn columns(value: &Json) -> BTreeMap<String, JsonData> {
    match value {
        Json::Object(entries) => entries
            .iter()
            .map(|(name, item)| (name.clone(), json_data(item)))
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            BTreeMap::new()
        }
    }
}

fn json_data(value: &Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(*flag),
        Json::Number(number) => JsonData::Number(number.clone()),
        Json::Text(text) => JsonData::String(text.clone()),
        Json::List(items) => JsonData::Array(items.iter().map(json_data).collect()),
        Json::Object(entries) => JsonData::Object(
            entries
                .iter()
                .map(|(name, item)| (name.clone(), json_data(item)))
                .collect(),
        ),
    }
}

async fn router_alias_answer(input: &Input) -> Value {
    let already_resolved = matches!(input.marker.unwrap_or_default(), MarkerState::Resolved);
    let eligible = matches!(input.route_class.unwrap_or_default(), RouteClass::LlmApi)
        && matches!(input.router.unwrap_or_default(), RouterState::Available);
    let state = match (eligible, already_resolved) {
        (false, _) => RouterAliasState::Ineligible { already_resolved },
        (true, true) => RouterAliasState::Resolved,
        (true, false) => RouterAliasState::Fresh,
    };
    let alias_input = RouterAliasInput {
        state,
        passthrough: matches!(
            input.dispatch.unwrap_or_default(),
            DispatchState::Passthrough
        ),
        requested_model: input.requested_model.as_deref(),
        raw_body_available: matches!(input.raw_body.unwrap_or_default(), RawBodyState::Available),
        key_router_settings: input.settings.as_ref(),
        team_id: None,
        client_requested_model: input.client_requested_model.as_deref(),
    };
    let rewrite = resolve_router_settings_model_group_alias(&alias_input, &NoTeams).await;
    rewrite_view(&rewrite)
}

fn rewrite_view(rewrite: &RouterAliasRewrite) -> Value {
    json!({
        "marker_set": rewrite.marker_set,
        "client_requested_model": rewrite.client_requested_model,
        "target": rewrite.target,
    })
}
