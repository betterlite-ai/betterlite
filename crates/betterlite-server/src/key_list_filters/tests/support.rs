use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError,
};
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::auth_team_lookup::ports::{TeamCache, TeamCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use secrecy::ExposeSecret as _;
use subtle::ConstantTimeEq as _;

use crate::key_list_filters::error::{KeyListFiltersError, KeyListReadError};
use crate::key_list_filters::ports::KeyListFiltersStore;
use crate::management_admins::ports::{TeamReadError, TeamReads};
use crate::team_access::caller::TeamCaller;

// the UserAPIKeyAuth fields a case sets, read through the same serde form production reads
pub(crate) fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(id) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(id.to_owned()));
    }
    if let Some(role) = user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_string()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(crate) fn team_caller(user_id: Option<&str>) -> TeamCaller {
    TeamCaller::try_from(&caller(user_id, Some(UserRole::InternalUser))).unwrap()
}

pub(crate) fn user_row(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn key_row(text: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn record(text: &str) -> TeamRecord {
    TeamRecord::parse(text.as_bytes()).unwrap().unwrap()
}

pub(crate) fn team(text: &str) -> LiteLLMTeamTable {
    decode_team_record(Some(&record(text))).unwrap().unwrap()
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn params(values: &[&str]) -> Vec<JsonData> {
    values
        .iter()
        .map(|value| JsonData::String((*value).to_owned()))
        .collect()
}

fn read_failure() -> KeyListFiltersError {
    // every store failure is indistinguishable to validate_key_list_check, so the fake reuses one
    KeyListFiltersError::Read(KeyListReadError::Teams(TeamReadError::Record(
        TeamRecordError::NotAnObject,
    )))
}

// the rows a key-list read returns, and the ordered trace of what was asked for
#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) user: Option<LiteLLMUserTable>,
    pub(crate) key: Option<LiteLLMVerificationToken>,
    pub(crate) key_fails: bool,
    pub(crate) reads: Mutex<Vec<String>>,
}

impl FakeStore {
    pub(crate) fn with_user(row: LiteLLMUserTable) -> Self {
        Self {
            user: Some(row),
            key: None,
            key_fails: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_key(mut self, row: LiteLLMVerificationToken) -> Self {
        self.key = Some(row);
        self
    }

    pub(crate) fn failing_key(mut self) -> Self {
        self.key_fails = true;
        self
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl KeyListFiltersStore for FakeStore {
    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError> {
        self.reads.lock().unwrap().push(format!("user {user_id}"));
        Ok(self.user.clone().filter(|row| row.user_id == user_id))
    }

    async fn key_by_hash(
        &self,
        key_hash: &KeyHash,
    ) -> Result<Option<LiteLLMVerificationToken>, KeyListFiltersError> {
        // the trace carries no credential, only that a key read happened
        self.reads.lock().unwrap().push("key".to_owned());
        if self.key_fails {
            return Err(read_failure());
        }
        Ok(self.key.clone().filter(|row| {
            row.token.as_ref().is_some_and(|stored| {
                bool::from(
                    stored
                        .expose_secret()
                        .as_bytes()
                        .ct_eq(key_hash.expose_secret().as_bytes()),
                )
            })
        }))
    }
}

// the scripted team rows in the order B724 would return them
#[derive(Default)]
pub(crate) struct FakeTeams {
    pub(crate) records: Vec<TeamRecord>,
    pub(crate) fail: bool,
    pub(crate) reads: Mutex<Vec<Vec<String>>>,
}

impl FakeTeams {
    pub(crate) fn with_records(texts: &[&str]) -> Self {
        Self {
            records: texts.iter().map(|text| record(text)).collect(),
            fail: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<Vec<String>> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl TeamReads for FakeTeams {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        self.reads.lock().unwrap().push(team_ids.to_vec());
        if self.fail {
            return Err(TeamReadError::Record(TeamRecordError::NotAnObject));
        }
        Ok(self.records.clone())
    }
}

// these cases never reach a team lookup, so the cache answers empty and records what it was asked
#[derive(Default)]
pub(crate) struct NoTeamReads {
    pub(crate) reads: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamCache for NoTeamReads {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.reads.lock().unwrap().push(key.to_owned());
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

pub(crate) fn team_lookup() -> (TeamLookup, Arc<NoTeamReads>) {
    let reads = Arc::new(NoTeamReads::default());
    let cache: Arc<dyn TeamCache> = Arc::<NoTeamReads>::clone(&reads);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::<NoTeamReads>::clone(&reads);
    let lookup = TeamLookup {
        cache,
        permission_cache,
        database: None,
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    };
    (lookup, reads)
}
