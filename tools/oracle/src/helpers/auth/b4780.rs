use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::agent_access::humans::{
    DelegatedTeamRequest, resolve_delegated_agent_team, verified_human_agent_grants,
};
use betterlite_auth::agent_access::listing::accessible_agents;
use betterlite_auth::agent_access::resolver::AgentAccessResolver;
use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use ports::{CaseGrants, CaseHumans, CaseResolver, Holder, NoTargets, key, source_resolver};

mod ports;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Accessible {
        agents: Vec<AgentFields>,
        user_role: Option<String>,
        contexts: Vec<ContextFields>,
        access: BTreeMap<String, Grant>,
    },
    HumanGrants {
        user_id: Option<String>,
        team_id: Option<String>,
        sources: Vec<SourceFields>,
    },
    DelegatedTeam {
        user_id: Option<String>,
        team_id: Option<String>,
        sources: Vec<SourceFields>,
        agent_id: String,
        explicit_team: bool,
        allowed_team_ids: Option<BTreeSet<String>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentFields {
    agent_id: String,
    identity_managed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextFields {
    user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Everything {
    Unrestricted,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Grant {
    Every(Everything),
    Only(BTreeSet<String>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceFields {
    team_id: Option<String>,
    agent_ids: BTreeSet<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        match input {
            Input::Accessible {
                agents,
                user_role,
                contexts,
                access,
            } => runtime.block_on(accessible(agents, user_role.as_deref(), contexts, access)),
            Input::HumanGrants {
                user_id,
                team_id,
                sources,
            } => human_grants(&runtime, user_id.as_deref(), team_id.as_deref(), sources),
            Input::DelegatedTeam {
                user_id,
                team_id,
                sources,
                agent_id,
                explicit_team,
                allowed_team_ids,
            } => {
                let resolver = source_resolver(user_id.as_deref(), sources);
                let request = DelegatedTeamRequest {
                    user_id: user_id.as_deref(),
                    agent_id,
                    team_id: team_id.as_deref(),
                    explicit_team: *explicit_team,
                    allowed_team_ids: allowed_team_ids.as_ref(),
                };
                delegated_team(&runtime, &resolver, &request)
            }
        }
    }
}

fn human_grants(
    runtime: &Runtime,
    user_id: Option<&str>,
    team_id: Option<&str>,
    sources: &[SourceFields],
) -> HelperResult {
    let resolver = source_resolver(user_id, sources);
    let granted = runtime
        .block_on(verified_human_agent_grants(&resolver, user_id, team_id))
        .or_value_error()?;
    Ok(json!(granted))
}

fn delegated_team(
    runtime: &Runtime,
    resolver: &CaseResolver<'_>,
    request: &DelegatedTeamRequest<'_>,
) -> HelperResult {
    match runtime.block_on(resolve_delegated_agent_team(resolver, request)) {
        Ok(team) => Ok(json!(team)),
        Err(error) => Ok(json!({"status": error.status(), "detail": error.to_string()})),
    }
}

async fn accessible(
    agents: &[AgentFields],
    user_role: Option<&str>,
    contexts: &[ContextFields],
    access: &BTreeMap<String, Grant>,
) -> HelperResult {
    let mut fields = BTreeMap::new();
    if let Some(role) = user_role {
        fields.insert(String::from("user_role"), JsonData::String(role.to_owned()));
    }
    let lister = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let mut context_auths = Vec::new();
    for context in contexts {
        context_auths.push(key(Some(&context.user_id), None).or_value_error()?);
    }
    let grants = case_grants(access);
    let mut listed_agents = Vec::new();
    for agent_fields in agents {
        listed_agents.push(listed_agent(agent_fields)?);
    }
    let resolver = AgentAccessResolver {
        registry: Arc::new(AgentRegistry::default()),
        grants: CaseGrants { access: grants },
        targets: NoTargets,
        humans: CaseHumans {
            contexts: context_auths,
            sources: &[],
        },
    };
    let shown = Box::pin(accessible_agents(&resolver, &lister, Some(listed_agents)))
        .await
        .or_value_error()?;
    let agent_ids: Vec<&str> = shown.iter().map(|agent| agent.agent_id.as_str()).collect();
    Ok(json!(agent_ids))
}

fn case_grants(access: &BTreeMap<String, Grant>) -> BTreeMap<Holder, AgentAccess> {
    access
        .iter()
        .map(|(user_id, grant)| {
            let context_holder = Holder {
                user_id: Some(user_id.clone()),
                team_id: None,
            };
            let granted = match grant {
                Grant::Every(Everything::Unrestricted) => AgentAccess::Unrestricted,
                Grant::Only(agent_ids) => AgentAccess::Restricted(agent_ids.clone()),
            };
            (context_holder, granted)
        })
        .collect()
}

fn listed_agent(fields: &AgentFields) -> Result<AgentResponse, Box<HelperError>> {
    serde_json::from_value(json!({
        "agent_id": fields.agent_id,
        "agent_name": fields.agent_id,
        "agent_card_params": {},
        "identity_managed": fields.identity_managed,
    }))
    .or_value_error()
}
