use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroups;
use betterlite_auth::agent_permission_handler::AgentGrants;
use betterlite_auth::agent_permission_handler::AgentGrantsApi as _;
use betterlite_auth::agent_permission_handler::error::GrantError;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_auth::auth_prefetch::permission::lookup::ObjectPermissionCache;
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{TeamCache, TeamStore};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, CadenceClock};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod ports;
mod tests;

// the case names its helper operation so a case copied under another helper fails to parse
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Grants {
        method: Method,
        strict: bool,
        database: bool,
        key: Option<KeySpec>,
        team: Option<GroupSpec>,
        db_agents: Vec<DbAgent>,
        groups: BTreeMap<String, Vec<String>>,
        config_agents: Vec<Json>,
        fail: Option<Failure>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Method {
    Key,
    KeyTeam,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Table,
    Team,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeySpec {
    team_id: Option<String>,
    agents: Vec<String>,
    agent_access_groups: Vec<String>,
    access_group_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupSpec {
    agents: Vec<String>,
    agent_access_groups: Vec<String>,
    access_group_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DbAgent {
    agent_id: String,
    agent_access_groups: Vec<String>,
}

// one case's reads behind every port AgentGrants takes, so the scripted rows alone decide the answer
#[derive(Clone, Debug)]
struct CaseWorld {
    database: bool,
    groups: BTreeMap<String, Vec<String>>,
    db_agents: Vec<DbAgent>,
    fail: Option<Failure>,
    team: Option<JsonData>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let Input::Grants {
            method,
            strict,
            database,
            key,
            team,
            db_agents,
            groups,
            config_agents,
            fail,
        } = input;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let case = CaseWorld {
            database: *database,
            groups: groups.clone(),
            db_agents: db_agents.clone(),
            fail: *fail,
            team: team.as_ref().map(team_row).transpose()?,
        };
        let grants = agent_grants(config_agents, &case)?;
        let auth = key_auth(key.as_ref())?;
        let access = match method {
            Method::Key => {
                runtime.block_on(grants.get_allowed_agents_for_key(auth.as_ref(), *strict))
            }
            Method::KeyTeam => {
                runtime.block_on(grants.resolve_key_team_agent_access(auth.as_ref(), *strict))
            }
        };
        match access {
            Ok(AgentAccess::Unrestricted) => Ok(json!("unrestricted")),
            Ok(AgentAccess::Restricted(agent_ids)) => Ok(json!(agent_ids)),
            Err(error) => Err(Box::new(grant_error(&error))),
        }
    }
}

fn agent_grants(
    config_agents: &[Json],
    case: &CaseWorld,
) -> Result<AgentGrants<CaseWorld, CaseWorld, CaseWorld>, Box<HelperError>> {
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some(config_agents))
        .or_value_error()?;
    let world = case.clone();
    let clock: Arc<dyn CadenceClock> = Arc::new(world.clone());
    let cache: Arc<dyn TeamCache> = Arc::new(world.clone());
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::new(world.clone());
    let store: Arc<dyn TeamStore> = Arc::new(world.clone());
    let reader: Arc<dyn ObjectPermissionStore> = Arc::new(world.clone());
    let writer: Arc<dyn ObjectPermissionStore> = Arc::new(world.clone());
    let stored = TeamDatabase {
        store,
        permission_reader: reader,
        permission_writer: writer,
    };
    let teams = TeamLookup {
        cache,
        permission_cache,
        database: case.database.then_some(stored),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, clock)),
        in_memory_ttl: 60,
    };
    Ok(AgentGrants::new(
        Arc::new(registry),
        AgentAccessGroups {
            reads: world.clone(),
        },
        teams,
        case.database.then_some(world.clone()),
        world,
    ))
}

fn team_row(team: &GroupSpec) -> Result<JsonData, Box<HelperError>> {
    let row = json!({
        "team_id": "t1",
        "object_permission": {
            "object_permission_id": "team-permission",
            "agents": team.agents,
            "agent_access_groups": team.agent_access_groups,
        },
        "access_group_ids": team.access_group_ids,
    });
    serde_json::from_value(row).or_value_error()
}

fn key_auth(spec: Option<&KeySpec>) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let mut fields = BTreeMap::new();
    if let Some(team_id) = spec.team_id.as_deref() {
        fields.insert(
            String::from("team_id"),
            JsonData::String(team_id.to_owned()),
        );
    }
    let permission = json!({
        "object_permission_id": "key-permission",
        "agents": spec.agents,
        "agent_access_groups": spec.agent_access_groups,
    });
    fields.insert(
        String::from("object_permission"),
        serde_json::from_value(permission).or_value_error()?,
    );
    if !spec.access_group_ids.is_empty() {
        let unified: JsonData =
            serde_json::from_value(json!(spec.access_group_ids)).or_value_error()?;
        fields.insert(String::from("access_group_ids"), unified);
    }
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    Ok(Some(auth))
}

fn grant_error(error: &GrantError) -> HelperError {
    HelperError {
        class: String::from("HTTPException"),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}
