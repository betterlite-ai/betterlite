use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use serde::Serialize;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::agent_access_groups::ceiling::AgentAccessGroups;
use crate::agent_access_groups::reads::AccessGroupReads;
use crate::agent_permission_handler::AgentGrants;
use crate::agent_permission_handler::error::GrantError;
use crate::agent_permission_handler::reads::AgentTableReads;
use crate::agent_policy::caller::AgentCaller;
use crate::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use crate::auth_model_patterns::access_groups::{
    AccessGroupError, AccessGroupLookup, AccessGroupResources,
};
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_read::table::AuthReadError;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use crate::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamStore, TeamStoreError,
};
use crate::auth_user_lookup::cadence::{Cadence, CadenceClock};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

// the scripted reads every collaborator of AgentGrants answers from
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) database: bool,
    pub(crate) team: Option<JsonData>,
    pub(crate) team_fails: bool,
    pub(crate) team_reads: Vec<(String, bool)>,
    pub(crate) rows: Vec<(String, Vec<String>)>,
    pub(crate) table_fails: bool,
    pub(crate) table_reads: Vec<(Vec<String>, bool)>,
    pub(crate) groups: BTreeMap<String, AccessGroupTable>,
    pub(crate) group_reads: Vec<(String, bool)>,
    pub(crate) agent_groups: BTreeMap<String, Vec<String>>,
    pub(crate) now: Duration,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct World {
    state: Arc<Mutex<State>>,
}

impl World {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn world(database: bool) -> World {
    let world = World::default();
    let mut state = world.lock();
    state.database = database;
    state.now = Duration::from_secs(100);
    drop(state);
    world
}

pub(crate) fn data<T: Serialize>(value: T) -> JsonData {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn registry(entries: &[&str]) -> Arc<AgentRegistry> {
    let parsed: Vec<Json> = entries
        .iter()
        .map(|entry| serde_json::from_str(entry).unwrap())
        .collect();
    let registry = AgentRegistry::default();
    registry.load_agents_from_config(Some(&parsed)).unwrap();
    Arc::new(registry)
}

// AgentGrants over one scripted World, wired the way the server wires the ports
pub(crate) fn handler(
    world: &World,
    registry: &Arc<AgentRegistry>,
) -> AgentGrants<World, World, World> {
    let database = world.lock().database;
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
        database: database.then_some(stored),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, clock)),
        in_memory_ttl: 60,
    };
    AgentGrants::new(
        Arc::clone(registry),
        AgentAccessGroups {
            reads: world.clone(),
        },
        teams,
        database.then_some(world.clone()),
        world.clone(),
    )
}

pub(crate) fn key_auth(
    team_id: Option<&str>,
    permission: Option<(&[&str], &[&str])>,
    unified: &[&str],
) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(team) = team_id {
        fields.insert(String::from("team_id"), JsonData::String(team.to_owned()));
    }
    if let Some((agents, groups)) = permission {
        let permission = serde_json::json!({
            "object_permission_id": "key-permission",
            "agents": agents,
            "agent_access_groups": groups,
        });
        fields.insert(String::from("object_permission"), data(permission));
    }
    if !unified.is_empty() {
        fields.insert(String::from("access_group_ids"), data(unified));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

// a key set by the server past the validator, so the caller reaches the team path
pub(crate) fn caller_auth(team_id: &str) -> UserAPIKeyAuth {
    let mut auth = key_auth(None, None, &[]);
    auth.extra.agent_caller = Some(AgentCaller {
        user_id: Some(String::from("caller")),
        team_id: Some(team_id.to_owned()),
    });
    auth
}

pub(crate) fn agent_auth(agent_id: &str) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    fields.insert(
        String::from("agent_id"),
        JsonData::String(agent_id.to_owned()),
    );
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(crate) fn team_row(agents: &[&str], groups: &[&str], unified: &[&str]) -> JsonData {
    data(serde_json::json!({
        "team_id": "t1",
        "object_permission": {
            "object_permission_id": "team-permission",
            "agents": agents,
            "agent_access_groups": groups,
        },
        "access_group_ids": unified,
    }))
}

pub(crate) fn bare_team() -> JsonData {
    data(serde_json::json!({"team_id": "t1"}))
}

pub(crate) fn group_row(access_group_id: &str, agents: &[&str]) -> AccessGroupTable {
    AccessGroupTable {
        access_group_id: access_group_id.to_owned(),
        access_group_name: access_group_id.to_owned(),
        description: None,
        access_model_names: Vec::new(),
        access_mcp_server_ids: Vec::new(),
        access_agent_ids: strings(agents),
        assigned_team_ids: Vec::new(),
        assigned_key_ids: Vec::new(),
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

pub(crate) fn assert_policy_unavailable(error: &GrantError) {
    assert_eq!(error.status(), Some(503));
    assert_eq!(error.to_string(), "Agent invocation policy is unavailable");
    let GrantError::PolicyUnavailable(_) = error else {
        panic!("policy unavailable expected, got {error:?}");
    };
}

fn store_failure() -> TeamStoreError {
    TeamStoreError::Read(Box::new(AuthReadError::Repository(DbError::NoDbConnected)))
}

impl AccessGroupReads for World {
    fn has_database(&self) -> bool {
        self.lock().database
    }

    fn agent_access_group_ids(&self, agent_id: &str) -> impl Future<Output = Vec<String>> + Send {
        let ids = self.lock().agent_groups.get(agent_id).cloned();
        ready(ids.unwrap_or_default())
    }

    fn access_group(
        &self,
        access_group_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send {
        let found = {
            let mut state = self.lock();
            state
                .group_reads
                .push((access_group_id.to_owned(), check_db_only));
            state.groups.get(access_group_id).cloned()
        };
        let Some(group) = found else {
            let error = TeamLookupError::AccessGroupNotFound {
                access_group_id: access_group_id.to_owned(),
            };
            return ready(Err(error));
        };
        ready(Ok(group))
    }
}

impl AgentTableReads for World {
    fn has_database(&self) -> bool {
        self.lock().database
    }

    fn agent_ids_with_access_groups(
        &self,
        groups: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<Vec<String>, DbError>> + Send {
        let answer: Result<Vec<String>, DbError> = {
            let mut state = self.lock();
            state.table_reads.push((groups.to_vec(), check_db_only));
            if state.table_fails {
                Err(DbError::NoDbConnected)
            } else {
                let rows = state
                    .rows
                    .iter()
                    .filter(|(_, declared)| declared.iter().any(|group| groups.contains(group)))
                    .map(|(agent_id, _)| agent_id.clone())
                    .collect();
                Ok(rows)
            }
        };
        ready(answer)
    }
}

impl AccessGroupLookup for World {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        let found = self.lock().groups.get(access_group_id).cloned();
        let Some(group) = found else {
            return Err(AccessGroupError::Unavailable {
                access_group_id: access_group_id.to_owned(),
            });
        };
        Ok(AccessGroupResources {
            agent_ids: group.access_agent_ids,
            ..AccessGroupResources::default()
        })
    }
}

#[async_trait]
impl TeamCache for World {
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
impl TeamStore for World {
    async fn find_team(&self, team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        let mut state = self.lock();
        state.team_reads.push((team_id.to_owned(), use_writer));
        if state.team_fails {
            return Err(store_failure());
        }
        Ok(state.team.clone())
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
impl ObjectPermissionCache for World {
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
impl ObjectPermissionStore for World {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

impl CadenceClock for World {
    fn now(&self) -> Duration {
        self.lock().now
    }
}

// the warnings this module logged, counted so a UI-team failure can be shown silent
#[derive(Clone, Default)]
pub(crate) struct Warnings {
    seen: Arc<Mutex<Vec<String>>>,
}

impl Warnings {
    pub(crate) fn install(&self) -> DefaultGuard {
        set_default(tracing_subscriber::registry().with(self.clone()))
    }

    pub(crate) fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata.level() == &Level::WARN
            && metadata
                .target()
                .starts_with("betterlite_auth::agent_permission_handler")
        {
            self.seen
                .lock()
                .unwrap()
                .push(String::from(metadata.target()));
        }
    }
}
