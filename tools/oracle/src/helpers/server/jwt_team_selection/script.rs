use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

// the scripted rows and events the Rust side of the case reads, mirroring the Python fakes
#[derive(Default)]
pub(super) struct Script {
    pub(super) teams: BTreeMap<String, JsonData>,
    pub(super) aliases: BTreeMap<String, Vec<JsonData>>,
    pub(super) events: Mutex<Vec<String>>,
}

impl Script {
    pub(super) fn events(&self) -> Vec<String> {
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
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
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

pub(super) fn lookup(script: &Arc<Script>, database: bool) -> TeamLookup {
    let cache: Arc<dyn TeamCache> = Arc::clone(script);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::clone(script);
    let store: Arc<dyn TeamStore> = Arc::clone(script);
    let reader: Arc<dyn ObjectPermissionStore> = Arc::clone(script);
    let writer: Arc<dyn ObjectPermissionStore> = Arc::clone(script);
    TeamLookup {
        cache,
        permission_cache,
        database: database.then(|| TeamDatabase {
            store,
            permission_reader: Arc::clone(&reader),
            permission_writer: writer,
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(1024, Arc::new(WallClock))),
        in_memory_ttl: 0,
    }
}
