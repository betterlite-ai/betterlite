use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::agent_access::error::AgentAccessError;
use betterlite_auth::agent_access::humans::{
    DelegatedTeamRequest, resolve_delegated_agent_team, verified_human_agent_grants,
};
use betterlite_auth::agent_access::listing::accessible_agents;
use betterlite_auth::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use betterlite_auth::agent_access::resolver::AgentAccessResolver;
use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::identity::AgentIdentityFailure;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

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

// the user and team a fake port reads back from an auth, whose fields betterlite-auth keeps private
#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Holder {
    user_id: Option<String>,
    team_id: Option<String>,
}

fn holder(auth: &UserAPIKeyAuth) -> Result<Holder, AgentAccessError> {
    serde_json::to_string(auth)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(|error| AgentAccessError::Rejected {
            status: 500,
            detail: error.to_string(),
        })
}

fn key(user_id: Option<&str>, team_id: Option<&str>) -> Result<UserAPIKeyAuth, AuthError> {
    let mut fields = BTreeMap::new();
    for (name, value) in [("user_id", user_id), ("team_id", team_id)] {
        if let Some(text) = value {
            fields.insert(name.to_owned(), JsonData::String(text.to_owned()));
        }
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

// LiteLLM's resolve_access, and for a human source its key and team grants: Restricted, so strict mode keeps it
struct CaseGrants {
    access: BTreeMap<Holder, AgentAccess>,
}

impl GrantSources for CaseGrants {
    async fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        let Some(given) = auth else {
            return Ok(AgentAccess::Unrestricted);
        };
        let found = holder(given)?;
        self.access
            .get(&found)
            .cloned()
            .ok_or_else(|| AgentAccessError::Rejected {
                status: 500,
                detail: format!("the case grants nothing to {found:?}"),
            })
    }

    async fn caller_access(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        Ok(AgentAccess::Unrestricted)
    }

    async fn key_access_strict(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<AgentAccess, AgentAccessError> {
        Ok(AgentAccess::Unrestricted)
    }

    async fn access_group_ceiling(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<Option<BTreeSet<String>>, AgentAccessError> {
        Ok(None)
    }

    async fn managed_ceilings(
        &self,
        _agent_id: &str,
        _access_group_ids: &[String],
    ) -> Result<Vec<BTreeSet<String>>, AgentAccessError> {
        Ok(Vec::new())
    }
}

struct NoTargets;

impl TargetAgents for NoTargets {
    fn has_database(&self) -> bool {
        false
    }

    async fn agent(&self, _agent_id: &str) -> Result<Option<AgentResponse>, AgentIdentityFailure> {
        Ok(None)
    }
}

// LiteLLM's effective_contexts, reload_admitted_user and admitted_subject_sources
struct CaseHumans<'case> {
    contexts: Vec<UserAPIKeyAuth>,
    sources: &'case [SourceFields],
}

impl HumanSources for CaseHumans<'_> {
    async fn reload_admitted_key(
        &self,
        _key_hash: &SecretString,
        _check_db_only: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        Err(AgentAccessError::Rejected {
            status: 500,
            detail: String::from("the cases reload no key"),
        })
    }

    async fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        if !requires_fresh_policy {
            return Err(AgentAccessError::Rejected {
                status: 500,
                detail: String::from("the human is reloaded without a fresh policy"),
            });
        }
        key(Some(user_id), None).map_err(|error| AgentAccessError::Rejected {
            status: 500,
            detail: error.to_string(),
        })
    }

    async fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        let user_id = holder(human)?.user_id;
        let admitted = self.sources.iter().filter(|source| {
            source.team_id.as_ref().is_none_or(|team_id| {
                allowed_team_ids.is_none_or(|allowed| allowed.contains(team_id))
            })
        });
        admitted
            .map(|source| key(user_id.as_deref(), source.team_id.as_deref()))
            .collect::<Result<Vec<UserAPIKeyAuth>, AuthError>>()
            .map_err(|error| AgentAccessError::Rejected {
                status: 500,
                detail: error.to_string(),
            })
    }

    async fn effective_contexts(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        Ok(self.contexts.clone())
    }
}

type CaseResolver<'case> = AgentAccessResolver<CaseGrants, NoTargets, CaseHumans<'case>>;

fn source_resolver<'case>(
    user_id: Option<&str>,
    sources: &'case [SourceFields],
) -> CaseResolver<'case> {
    let access = sources
        .iter()
        .map(|source| {
            let source_holder = Holder {
                user_id: user_id.map(str::to_owned),
                team_id: source.team_id.clone(),
            };
            (
                source_holder,
                AgentAccess::Restricted(source.agent_ids.clone()),
            )
        })
        .collect();
    AgentAccessResolver {
        registry: Arc::new(AgentRegistry::default()),
        grants: CaseGrants { access },
        targets: NoTargets,
        humans: CaseHumans {
            contexts: Vec::new(),
            sources,
        },
    }
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
            } => {
                let resolver = source_resolver(user_id.as_deref(), sources);
                let granted = runtime
                    .block_on(verified_human_agent_grants(
                        &resolver,
                        user_id.as_deref(),
                        team_id.as_deref(),
                    ))
                    .or_value_error()?;
                Ok(json!(granted))
            }
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
                match runtime.block_on(resolve_delegated_agent_team(&resolver, &request)) {
                    Ok(team) => Ok(json!(team)),
                    Err(error) => {
                        Ok(json!({"status": error.status(), "detail": error.to_string()}))
                    }
                }
            }
        }
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
    let mut grants = BTreeMap::new();
    for context in contexts {
        context_auths.push(key(Some(&context.user_id), None).or_value_error()?);
    }
    for (user_id, grant) in access {
        let context_holder = Holder {
            user_id: Some(user_id.clone()),
            team_id: None,
        };
        let granted = match grant {
            Grant::Every(Everything::Unrestricted) => AgentAccess::Unrestricted,
            Grant::Only(agent_ids) => AgentAccess::Restricted(agent_ids.clone()),
        };
        grants.insert(context_holder, granted);
    }
    let mut listed_agents = Vec::new();
    for fields in agents {
        listed_agents.push(listed_agent(fields)?);
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
    let listed = accessible_agents(&resolver, &lister, Some(listed_agents))
        .await
        .or_value_error()?;
    let agent_ids: Vec<&str> = listed.iter().map(|agent| agent.agent_id.as_str()).collect();
    Ok(json!(agent_ids))
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
