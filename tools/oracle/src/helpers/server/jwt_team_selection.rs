use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_keys::agent::AgentLookup;
use betterlite_auth::jwt_team_selection::agent::resolve_agent_id;
use betterlite_auth::jwt_team_selection::candidates::{
    CandidateContext, CandidateRequest, find_team_with_model_access,
};
use betterlite_auth::jwt_team_selection::claims_teams::find_and_validate_specific_team_id;
use betterlite_auth::jwt_team_selection::error::TeamSelectionError;
use betterlite_auth::jwt_team_selection::header::resolve_team_from_header;
use betterlite_auth::jwt_team_selection::object::validate_object_id;
use betterlite_auth::jwt_team_selection::settings::SelectionSettings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_paths::paths::Document;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod case;
mod script;

use self::case::{Fields, Function, Input, Scenario, claim_settings, registry, selection_settings};
use self::script::{Script, lookup};

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

pub(crate) struct Op;

// the litellm_jwtauth fields the selection reads, named as LiteLLM names them
impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let answers = input
            .scenarios
            .iter()
            .map(|scenario| runtime.block_on(answer(scenario)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"scenarios": answers}))
    }
}

async fn answer(scenario: &Scenario) -> HelperResult {
    let script = Arc::new(Script {
        teams: scenario.teams.clone(),
        aliases: scenario.aliases.clone(),
        events: std::sync::Mutex::new(Vec::new()),
    });
    let lookup = lookup(&script, scenario.database);
    let claims_settings = claim_settings(&scenario.fields);
    let document = Document {
        json: scenario.token.clone().unwrap_or(Value::Null),
    };
    let claims = JwtClaims {
        claims: document,
        settings: &claims_settings,
        trusted_issuers: &[],
    };
    let settings = selection_settings(&scenario.fields);
    match scenario.function {
        Function::Agent => {
            let registry = Registry::new(&scenario.agents)?;
            let answer =
                resolve_agent_id(&claims, &registry).map(|agent_id| json!({"agent_id": agent_id}));
            Ok(tagged(answer, &script))
        }
        Function::SpecificTeam => Ok(tagged(
            find_and_validate_specific_team_id(
                &claims,
                &lookup,
                &settings,
                scenario.fields.team_id_upsert.then_some(true),
            )
            .await
            .map(|(team_id, _team)| json!({"team_id": team_id})),
            &script,
        )),
        Function::Candidates => Ok(tagged(
            await_candidates(scenario, &claims, &lookup, &settings).await?,
            &script,
        )),
        Function::Header => Ok(tagged(
            resolve_header(scenario, &lookup, &settings).await,
            &script,
        )),
        Function::Object => Ok(tagged(
            validate_object_id(
                scenario.user_id.as_deref(),
                scenario.team_id.as_deref(),
                scenario.enforce_rbac,
                scenario.is_proxy_admin,
            )
            .map(|()| json!({"ok": true})),
            &script,
        )),
    }
}

async fn await_candidates(
    scenario: &Scenario,
    claims: &JwtClaims<'_>,
    lookup: &TeamLookup,
    settings: &SelectionSettings,
) -> Result<Value, TeamSelectionError> {
    let team_ids = get_all_team_ids(claims)?;
    let registry = registry(scenario);
    let aliases = BTreeMap::new();
    let models = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &UNRESOLVED_PROVIDER,
        max_recurse_depth: 100,
    };
    let deps = CandidateContext {
        lookup,
        registry: &registry,
        models: &models,
    };
    let request = CandidateRequest {
        team_ids: &team_ids,
        requested_model: scenario.requested_model.as_deref(),
        route: &scenario.route,
        request_method: scenario.request_method.as_deref(),
    };
    find_team_with_model_access(&request, &deps, settings)
        .await
        .map(|selected| json!({"team_id": selected.map(|(team_id, _row)| team_id)}))
}

async fn resolve_header(
    scenario: &Scenario,
    lookup: &TeamLookup,
    settings: &SelectionSettings,
) -> Result<Value, TeamSelectionError> {
    let allowed = scenario
        .allowed_team_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let header = resolve_team_from_header(
        Some(&scenario.headers),
        &allowed,
        settings.routing.fallback_to_db_teams,
        lookup,
    )
    .await?;
    Ok(match header {
        Some(team) => json!({"header_value": team.header_value, "team_id": team.team_id}),
        None => json!({"header_value": Value::Null, "team_id": Value::Null}),
    })
}

fn tagged(result: Result<Value, TeamSelectionError>, script: &Arc<Script>) -> Value {
    let events = json!(script.events());
    match result {
        Ok(value) => json!({"events": events, "value": value}),
        Err(error) => json!({"error": error_fields(error), "events": events}),
    }
}

fn error_fields(error: TeamSelectionError) -> Value {
    let exception = error.into_proxy_exception();
    json!({
        "status": exception.code.parse::<u16>().unwrap_or_default(),
        "message": exception.message,
        "type": exception.error_type,
        "param": exception.param.unwrap_or_else(|| "None".to_owned()),
        "code": exception.code,
    })
}

struct Registry(Vec<AgentResponse>);

impl Registry {
    fn new(rows: &[Value]) -> Result<Self, Box<HelperError>> {
        let agents = rows
            .iter()
            .cloned()
            .map(serde_json::from_value::<AgentResponse>)
            .collect::<Result<Vec<_>, _>>()
            .or_value_error()?;
        Ok(Self(agents))
    }
}

impl AgentLookup for Registry {
    fn get_agent_list(&self) -> Vec<AgentResponse> {
        self.0.clone()
    }

    fn get_agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0
            .iter()
            .find(|agent| agent.agent_id == agent_id)
            .cloned()
    }

    fn get_agent_by_name(&self, agent_name: &str) -> Option<AgentResponse> {
        self.0
            .iter()
            .find(|agent| agent.agent_name == agent_name)
            .cloned()
    }
}
