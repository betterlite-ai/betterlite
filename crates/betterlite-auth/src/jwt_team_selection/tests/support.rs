use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::jwt_settings::Settings as JwtSettings;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::agent_policy::AgentResponse;
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use crate::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamProvisioner, TeamSeed, TeamStore,
};
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::auth_user_lookup::ports::PortError;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::settings::JwtAuthSettings;
use crate::jwt_keys::agent::AgentLookup;
use crate::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};

use super::super::settings::SelectionSettings;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// every team read this module makes, served from the rows the test seeds
#[derive(Default)]
pub(crate) struct Script {
    pub(crate) teams: BTreeMap<String, JsonData>,
    pub(crate) aliases: BTreeMap<String, Vec<JsonData>>,
    pub(crate) provisioned: BTreeMap<String, JsonData>,
    pub(crate) events: Mutex<Vec<String>>,
}

impl Script {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn with_team(mut self, text: &str) -> Self {
        let row = json(text);
        let id = team_from(&row).fields.base.team_id.clone();
        self.teams.insert(id, row);
        self
    }

    // a row that does not validate, stored under the id the lookup asks for
    #[must_use]
    pub(crate) fn with_raw(mut self, team_id: &str, text: &str) -> Self {
        self.teams.insert(team_id.to_owned(), json(text));
        self
    }

    #[must_use]
    pub(crate) fn with_alias(mut self, alias: &str, rows: &[&str]) -> Self {
        self.aliases.insert(
            alias.to_owned(),
            rows.iter().map(|text| json(text)).collect(),
        );
        self
    }

    #[must_use]
    pub(crate) fn with_provisioned(mut self, text: &str) -> Self {
        let row = json(text);
        let id = team_from(&row).fields.base.team_id.clone();
        self.provisioned.insert(id, row);
        self
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[async_trait]
impl TeamCache for Script {
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
impl TeamStore for Script {
    async fn find_team(&self, team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.record(format!("team {team_id}"));
        Ok(self.teams.get(team_id).cloned())
    }

    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        self.record(format!("alias {team_alias}"));
        Ok(self.aliases.get(team_alias).cloned().unwrap_or_default())
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
impl ObjectPermissionStore for Script {
    async fn find_permission(
        &self,
        _id: &str,
    ) -> Result<Option<JsonData>, crate::auth_prefetch::permission::lookup::PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for Script {
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
        0.0
    }
}

#[async_trait]
impl TeamProvisioner for Script {
    async fn create(&self, seed: TeamSeed) -> Result<LiteLLMTeamTable, PortError> {
        self.record(format!("provision {}", seed.team_id));
        self.provisioned
            .get(&seed.team_id)
            .map(team_from)
            .ok_or_else(|| PortError::new("no provisioned row"))
    }
}

#[must_use]
pub(crate) fn lookup(script: &Arc<Script>, database: bool) -> TeamLookup {
    let cache: Arc<dyn TeamCache> = Arc::clone(script);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::clone(script);
    let store: Arc<dyn TeamStore> = Arc::clone(script);
    let reader: Arc<dyn ObjectPermissionStore> = Arc::clone(script);
    let writer: Arc<dyn ObjectPermissionStore> = Arc::clone(script);
    let provisioner: Arc<dyn TeamProvisioner> = Arc::clone(script);
    TeamLookup {
        cache,
        permission_cache,
        database: database.then(|| TeamDatabase {
            store,
            permission_reader: Arc::clone(&reader),
            permission_writer: writer,
        }),
        provisioner: Some(provisioner),
        cadence: Arc::new(Cadence::new(1024, Arc::new(WallClock))),
        in_memory_ttl: 0,
    }
}

// the claim readers and the selection settings share one JwtAuthSettings, as they do in production
pub(crate) fn settings<Patch>(patch: Patch) -> JwtAuthSettings
where
    Patch: FnOnce(&mut JwtAuthSettings),
{
    let mut settings = JwtAuthSettings::default();
    patch(&mut settings);
    settings
}

pub(crate) fn selection<Patch>(patch: Patch) -> SelectionSettings
where
    Patch: FnOnce(&mut SelectionSettings),
{
    let mut settings = SelectionSettings::from_section(&JwtSettings::default());
    patch(&mut settings);
    settings
}

pub(crate) fn claims<'auth>(
    token: serde_json::Value,
    settings: &'auth JwtAuthSettings,
) -> JwtClaims<'auth> {
    JwtClaims {
        claims: betterlite_paths::paths::Document { json: token },
        settings,
        trusted_issuers: &[],
    }
}

// the status, type and message B757 puts on the wire for a selection failure
#[must_use]
pub(crate) fn rendered(
    error: crate::jwt_team_selection::error::TeamSelectionError,
) -> (u16, String, String) {
    let exception = error.into_proxy_exception();
    let status = exception.code.parse().expect("the status is numeric");
    (
        status,
        exception.error_type.clone(),
        exception.message.clone(),
    )
}

#[must_use]
pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

#[must_use]
pub(crate) fn team(text: &str) -> LiteLLMTeamTable {
    team_from(&json(text))
}

fn team_from(row: &JsonData) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(row.clone()).expect("the team row validates")
}

#[must_use]
pub(crate) fn context(global_model_aliases: &BTreeMap<String, String>) -> ModelAccessContext<'_> {
    ModelAccessContext {
        view: None,
        global_model_aliases,
        compaction: None,
        provider: &UNRESOLVED_PROVIDER,
        max_recurse_depth: 100,
    }
}

#[must_use]
pub(crate) fn protected_registry(route: &str, method: &str) -> PassThroughRegistry {
    PassThroughRegistry {
        root_path: String::new(),
        registrations: vec![(
            format!("p1:exact:{route}:{method}"),
            PassThroughMetadata {
                endpoint_id: "p1".to_owned(),
                route_type: PassThroughRouteType::Exact,
                path: route.to_owned(),
                methods: vec![method.to_owned()],
                auth: Some(true),
            },
        )],
    }
}

#[must_use]
pub(crate) fn open_registry() -> PassThroughRegistry {
    PassThroughRegistry {
        root_path: String::new(),
        registrations: Vec::new(),
    }
}

#[must_use]
pub(crate) fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[must_use]
pub(crate) fn ids(values: &[&str]) -> std::collections::BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[derive(Default)]
pub(crate) struct Agents {
    pub(crate) agents: Vec<AgentResponse>,
}

impl Agents {
    #[must_use]
    pub(crate) fn with(mut self, text: &str) -> Self {
        self.agents
            .push(serde_json::from_str(text).expect("the agent row validates"));
        self
    }
}

impl AgentLookup for Agents {
    fn get_agent_list(&self) -> Vec<AgentResponse> {
        self.agents.clone()
    }

    fn get_agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.agents
            .iter()
            .find(|agent| agent.agent_id == agent_id)
            .cloned()
    }

    fn get_agent_by_name(&self, agent_name: &str) -> Option<AgentResponse> {
        self.agents
            .iter()
            .find(|agent| agent.agent_name == agent_name)
            .cloned()
    }
}
