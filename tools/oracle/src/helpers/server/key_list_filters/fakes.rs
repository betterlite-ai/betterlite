use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError,
};
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::auth_team_lookup::ports::{TeamCache, TeamCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_server::key_list_filters::error::{KeyListFiltersError, KeyListReadError};
use betterlite_server::key_list_filters::ports::KeyListFiltersStore;
use betterlite_server::management_admins::ports::{TeamReadError, TeamReads};
use secrecy::ExposeSecret as _;
use serde_json::Value;

// the rows a case scripts for the caller read and the key-hash read, plus the ordered call trace
pub(super) struct ScriptStore {
    user: Option<LiteLLMUserTable>,
    key: Option<LiteLLMVerificationToken>,
    key_fails: bool,
    trace: Mutex<Vec<String>>,
}

impl ScriptStore {
    pub(super) fn new(
        user: Option<LiteLLMUserTable>,
        key: Option<LiteLLMVerificationToken>,
        key_fails: bool,
    ) -> Self {
        Self {
            user,
            key,
            key_fails,
            trace: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn trace(&self) -> Vec<String> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn read_failure() -> KeyListFiltersError {
    KeyListFiltersError::Read(KeyListReadError::Teams(TeamReadError::Record(
        TeamRecordError::NotAnObject,
    )))
}

#[async_trait]
impl KeyListFiltersStore for ScriptStore {
    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("user {user_id}"));
        Ok(self.user.clone())
    }

    async fn key_by_hash(
        &self,
        key_hash: &KeyHash,
    ) -> Result<Option<LiteLLMVerificationToken>, KeyListFiltersError> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("key".to_owned());
        if self.key_fails {
            return Err(read_failure());
        }
        Ok(self.key.clone().filter(|row| {
            row.token
                .as_ref()
                .is_some_and(|stored| KeyHash::from(stored.expose_secret()) == *key_hash)
        }))
    }
}

// the team rows the single find_many returns; a row that is not a JSON object fails the read
pub(super) struct ScriptTeams {
    pub(super) rows: Vec<Value>,
    pub(super) fails: bool,
    pub(super) trace: Mutex<Vec<String>>,
}

impl ScriptTeams {
    pub(super) fn trace(&self) -> Vec<String> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TeamReads for ScriptTeams {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("teams {}", team_ids.join(",")));
        if self.fails {
            return Err(TeamReadError::Record(TeamRecordError::NotAnObject));
        }
        let records: Result<Vec<Option<TeamRecord>>, TeamRecordError> = self
            .rows
            .iter()
            .map(|row| TeamRecord::parse(row.to_string().as_bytes()))
            .collect();
        Ok(records
            .map_err(TeamReadError::Record)?
            .into_iter()
            .flatten()
            .collect())
    }
}

// a key's team access check never runs in these cases, so both caches answer a miss
pub(super) struct NoTeamReads;

#[async_trait]
impl TeamCache for NoTeamReads {
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
impl ObjectPermissionCache for NoTeamReads {
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

pub(super) fn team_lookup() -> TeamLookup {
    let cache: Arc<dyn TeamCache> = Arc::new(NoTeamReads);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::new(NoTeamReads);
    TeamLookup {
        cache,
        permission_cache,
        database: None,
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}
