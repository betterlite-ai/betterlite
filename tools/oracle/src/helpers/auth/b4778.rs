use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;

use betterlite_auth::agent_access_groups::access::{AgentAccess, intersect_agent_access};
use betterlite_auth::agent_access_groups::ceiling::{AgentAccessGroupCeiling, AgentAccessGroups};
use betterlite_auth::agent_access_groups::reads::AccessGroupReads;
use betterlite_auth::agent_caller::caller_auth::{agent_caller_auth, caller_record};
use betterlite_auth::agent_policy::caller::AgentCaller;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Ceiling {
        agent_id: String,
        agent_access_group_ids: Vec<String>,
        groups: BTreeMap<String, Option<GroupLists>>,
    },
    ManagedCeilings {
        access_group_ids: Vec<String>,
    },
    CallerAuth {
        agent_id: Option<String>,
        models: Vec<String>,
        agent_caller: Option<AgentCaller>,
        requires_fresh_policy: bool,
    },
    IntersectAccess {
        config_agents: Vec<Json>,
        key_access: Side,
        team_access: Side,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupLists {
    access_model_names: Vec<String>,
    access_mcp_server_ids: Vec<String>,
    access_agent_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Side {
    Unrestricted,
    Restricted {
        restricted_ids: Vec<String>,
        restricted_by_legacy_name: Vec<String>,
        restricted_by_stable_name: Vec<String>,
    },
}

// LiteLLM's injected loaders: a null group is one that did not load
struct CaseReads<'case> {
    database: bool,
    agent_id: &'case str,
    access_group_ids: &'case [String],
    groups: &'case BTreeMap<String, Option<GroupLists>>,
}

impl AccessGroupReads for CaseReads<'_> {
    fn has_database(&self) -> bool {
        self.database
    }

    fn agent_access_group_ids(&self, agent_id: &str) -> impl Future<Output = Vec<String>> + Send {
        let known = agent_id == self.agent_id;
        ready(if known {
            self.access_group_ids.to_vec()
        } else {
            Vec::new()
        })
    }

    fn access_group(
        &self,
        access_group_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send {
        let found = self.groups.get(access_group_id).cloned().flatten();
        ready(
            found
                .map(|lists| group_row(access_group_id, lists))
                .ok_or_else(|| TeamLookupError::AccessGroupNotFound {
                    access_group_id: access_group_id.to_owned(),
                }),
        )
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
            Input::Ceiling {
                agent_id,
                agent_access_group_ids,
                groups,
            } => {
                let reads = CaseReads {
                    database: true,
                    agent_id: agent_id.as_str(),
                    access_group_ids: agent_access_group_ids.as_slice(),
                    groups,
                };
                let resolver = AgentAccessGroups { reads };
                let ceiling = runtime
                    .block_on(resolver.resolve_agent_access_group_ceiling(agent_id, false))
                    .or_value_error()?;
                Ok(ceiling.as_ref().map_or(Value::Null, ceiling_view))
            }
            Input::ManagedCeilings { access_group_ids } => {
                let groups = BTreeMap::new();
                let reads = CaseReads {
                    database: false,
                    agent_id: "",
                    access_group_ids: &[],
                    groups: &groups,
                };
                let resolver = AgentAccessGroups { reads };
                let ceilings = runtime
                    .block_on(resolver.resolve_managed_agent_ceilings(access_group_ids))
                    .or_value_error()?;
                Ok(Value::Array(ceilings.iter().map(ceiling_view).collect()))
            }
            Input::CallerAuth {
                agent_id,
                models,
                agent_caller,
                requires_fresh_policy,
            } => caller_auth(
                agent_id.as_deref(),
                models,
                agent_caller.as_ref(),
                *requires_fresh_policy,
            ),
            Input::IntersectAccess {
                config_agents,
                key_access,
                team_access,
            } => intersect_access(config_agents, key_access, team_access),
        }
    }
}

fn group_row(access_group_id: &str, lists: GroupLists) -> AccessGroupTable {
    AccessGroupTable {
        access_group_id: access_group_id.to_owned(),
        access_group_name: access_group_id.to_owned(),
        description: None,
        access_model_names: lists.access_model_names,
        access_mcp_server_ids: lists.access_mcp_server_ids,
        access_agent_ids: lists.access_agent_ids,
        assigned_team_ids: Vec::new(),
        assigned_key_ids: Vec::new(),
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

fn ceiling_view(ceiling: &AgentAccessGroupCeiling) -> Value {
    json!({
        "access_group_ids": ceiling.access_group_ids,
        "models": ceiling.models,
        "mcp_server_ids": ceiling.mcp_server_ids,
        "agent_ids": ceiling.agent_ids,
    })
}

// requires_fresh_policy is private to betterlite-auth: the cases ignore it, Rust tests prove it
fn caller_auth(
    agent_id: Option<&str>,
    models: &[String],
    agent_caller: Option<&AgentCaller>,
    requires_fresh_policy: bool,
) -> HelperResult {
    let fields = BTreeMap::from([
        (
            "agent_id".to_owned(),
            agent_id.map_or(JsonData::Null, |id| JsonData::String(id.to_owned())),
        ),
        (
            "models".to_owned(),
            JsonData::Array(models.iter().cloned().map(JsonData::String).collect()),
        ),
    ]);
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let record = match agent_caller {
        None => agent_caller_auth(&key).or_value_error()?,
        Some(caller) => Some(caller_record(caller, requires_fresh_policy, None).or_value_error()?),
    };
    let Some(stand_in) = record else {
        return Ok(Value::Null);
    };
    let dumped = serde_json::to_value(&stand_in).or_value_error()?;
    Ok(json!({
        "user_id": dumped["user_id"],
        "team_id": dumped["team_id"],
        "agent_id": dumped["agent_id"],
        "models": dumped["models"],
    }))
}

fn intersect_access(config_agents: &[Json], key_access: &Side, team_access: &Side) -> HelperResult {
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some(config_agents))
        .or_value_error()?;
    let key = side_access(&registry, key_access)?;
    let team = side_access(&registry, team_access)?;
    match intersect_agent_access(&registry, &key, &team) {
        AgentAccess::Unrestricted => Ok(json!("unrestricted")),
        AgentAccess::Restricted(agent_ids) => Ok(json!(agent_ids)),
    }
}

fn side_access(registry: &AgentRegistry, side: &Side) -> Result<AgentAccess, Box<HelperError>> {
    let Side::Restricted {
        restricted_ids,
        restricted_by_legacy_name,
        restricted_by_stable_name,
    } = side
    else {
        return Ok(AgentAccess::Unrestricted);
    };
    let mut ids: BTreeSet<String> = restricted_ids.iter().cloned().collect();
    for name in restricted_by_stable_name {
        ids.insert(stable_id(registry, name)?);
    }
    for name in restricted_by_legacy_name {
        let stable = stable_id(registry, name)?;
        let aliases = registry.ids_for_agent(&stable);
        ids.extend(aliases.into_iter().filter(|id| *id != stable));
    }
    Ok(AgentAccess::Restricted(ids))
}

fn stable_id(registry: &AgentRegistry, name: &str) -> Result<String, Box<HelperError>> {
    registry
        .get_agent_by_name(name)
        .map(|agent| agent.agent_id)
        .ok_or_else(|| format!("no config agent is named {name}"))
        .or_value_error()
}
