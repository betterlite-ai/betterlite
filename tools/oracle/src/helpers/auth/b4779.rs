use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroups;
use betterlite_auth::agent_access_groups::reads::AccessGroupReads;
use betterlite_auth::agent_permission_handler::AgentGrants;
use betterlite_auth::agent_permission_handler::AgentGrantsApi as _;
use betterlite_auth::agent_permission_handler::error::GrantError;
use betterlite_auth::agent_permission_handler::reads::AgentTableReads;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_auth::auth_model_patterns::access_groups::{
    AccessGroupError, AccessGroupLookup, AccessGroupResources,
};
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamStore, TeamStoreError,
};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, CadenceClock};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    method: Method,
    strict: bool,
    database: bool,
    key: Option<KeySpec>,
    team: Option<GroupSpec>,
    db_agents: Vec<DbAgent>,
    groups: BTreeMap<String, Vec<String>>,
    config_agents: Vec<Json>,
    fail: Option<Failure>,
}

// the case names its helper operation so a case copied under another helper fails to parse
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Grants,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Method {
    Key,
    KeyTeam,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DbAgent {
    agent_id: String,
    agent_access_groups: Vec<String>,
}

// one case's reads behind every port AgentGrants takes, so the scripted rows alone decide the answer
#[derive(Clone, Debug)]
struct CaseWorld<'case> {
    case: &'case Input,
    team: Option<JsonData>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let grants = agent_grants(input)?;
        let key = key_auth(input.key.as_ref())?;
        let access = match input.method {
            Method::Key => {
                runtime.block_on(grants.get_allowed_agents_for_key(key.as_ref(), input.strict))
            }
            Method::KeyTeam => {
                runtime.block_on(grants.resolve_key_team_agent_access(key.as_ref(), input.strict))
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
    case: &Input,
) -> Result<AgentGrants<CaseWorld<'_>, CaseWorld<'_>, CaseWorld<'_>>, Box<HelperError>> {
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some(&case.config_agents))
        .or_value_error()?;
    let team = case.team.as_ref().map(team_row).transpose()?;
    let world = CaseWorld { case, team };
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

fn store_failure() -> TeamStoreError {
    TeamStoreError::Read(Box::new(AuthReadError::Repository(DbError::NoDbConnected)))
}

impl AccessGroupReads for CaseWorld<'_> {
    fn has_database(&self) -> bool {
        self.case.database
    }

    fn agent_access_group_ids(&self, _agent_id: &str) -> impl Future<Output = Vec<String>> + Send {
        ready(Vec::new())
    }

    fn access_group(
        &self,
        access_group_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send {
        let found = self.case.groups.get(access_group_id).cloned();
        let Some(agent_ids) = found else {
            let error = TeamLookupError::AccessGroupNotFound {
                access_group_id: access_group_id.to_owned(),
            };
            return ready(Err(error));
        };
        let row = AccessGroupTable {
            access_group_id: access_group_id.to_owned(),
            access_group_name: access_group_id.to_owned(),
            description: None,
            access_model_names: Vec::new(),
            access_mcp_server_ids: Vec::new(),
            access_agent_ids: agent_ids,
            assigned_team_ids: Vec::new(),
            assigned_key_ids: Vec::new(),
            created_at: None,
            created_by: None,
            updated_at: None,
            updated_by: None,
        };
        ready(Ok(row))
    }
}

impl AgentTableReads for CaseWorld<'_> {
    fn has_database(&self) -> bool {
        self.case.database
    }

    fn agent_ids_with_access_groups(
        &self,
        groups: &[String],
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Vec<String>, DbError>> + Send {
        let answer: Result<Vec<String>, DbError> = match self.case.fail {
            Some(Failure::Table) => Err(DbError::NoDbConnected),
            Some(Failure::Team) | None => Ok(self
                .case
                .db_agents
                .iter()
                .filter(|row| {
                    row.agent_access_groups
                        .iter()
                        .any(|group| groups.contains(group))
                })
                .map(|row| row.agent_id.clone())
                .collect()),
        };
        ready(answer)
    }
}

impl AccessGroupLookup for CaseWorld<'_> {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        let found = self.case.groups.get(access_group_id);
        let Some(agent_ids) = found else {
            return Err(AccessGroupError::Unavailable {
                access_group_id: access_group_id.to_owned(),
            });
        };
        Ok(AccessGroupResources {
            agent_ids: agent_ids.clone(),
            ..AccessGroupResources::default()
        })
    }
}

#[async_trait]
impl TeamCache for CaseWorld<'_> {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        Ok(None)
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl TeamStore for CaseWorld<'_> {
    async fn find_team(&self, _team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        if matches!(self.case.fail, Some(Failure::Team)) {
            return Err(store_failure());
        }
        Ok(self.team.clone())
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for CaseWorld<'_> {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl ObjectPermissionStore for CaseWorld<'_> {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

impl CadenceClock for CaseWorld<'_> {
    fn now(&self) -> Duration {
        Duration::from_secs(100)
    }
}
