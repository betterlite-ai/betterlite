use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::user_repository::projection::user_record_to_model;

use crate::management_admins::ports::{AdminUserReads, TeamReadError, TeamReads, UserReadError};
use crate::management_admins::scope::AdminCaller;
use crate::management_rbac::guard::AdminPrivileges;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the exemption lookup failed")]
pub(crate) struct LookupFailed;

pub(crate) type CallerRecord = (Option<String>, bool);

pub(crate) struct FakePrivileges {
    pub(crate) outcome: Result<bool, LookupFailed>,
    pub(crate) callers: Mutex<Vec<CallerRecord>>,
}

#[async_trait]
impl AdminPrivileges for FakePrivileges {
    type Failure = LookupFailed;

    async fn has_admin_privileges(&self, caller: &AdminCaller<'_>) -> Result<bool, LookupFailed> {
        let record = (caller.user_id.map(str::to_owned), caller.proxy_admin);
        self.callers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        self.outcome
    }
}

pub(crate) struct EmptyCache;

#[async_trait]
impl UserCache for EmptyCache {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

pub(crate) type UserRead = (Option<String>, Arc<dyn UserCache>);

pub(crate) struct FakeUsers {
    pub(crate) user: Option<LiteLLMUserTable>,
    pub(crate) reads: Mutex<Vec<UserRead>>,
}

#[async_trait]
impl AdminUserReads for FakeUsers {
    fn fresh_cache(&self) -> Result<Arc<dyn UserCache>, UserReadError> {
        Ok(Arc::new(EmptyCache))
    }

    async fn user_object(
        &self,
        request: UserLookupRequest<'_>,
        cache: Arc<dyn UserCache>,
    ) -> Result<Option<LiteLLMUserTable>, UserReadError> {
        let read = (request.user_id.map(str::to_owned), cache);
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(read);
        Ok(self.user.clone())
    }
}

pub(crate) struct FakeTeams {
    pub(crate) records: Vec<TeamRecord>,
    pub(crate) unreachable: bool,
    pub(crate) requested: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl TeamReads for FakeTeams {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_ids.to_vec());
        if self.unreachable {
            return Err(TeamReadError::Database(DbError::NoDbConnected));
        }
        Ok(self.records.clone())
    }
}

pub(crate) fn user_row(json: &str) -> LiteLLMUserTable {
    let record: JsonData = serde_json::from_str(json).expect("the fixture user is JSON");
    user_record_to_model(Some(&record))
        .expect("the fixture user row is readable")
        .expect("the fixture user row is present")
}

pub(crate) fn team_row(json: &str) -> TeamRecord {
    TeamRecord::parse(json.as_bytes())
        .expect("the fixture team parses")
        .expect("the fixture team is an object")
}
