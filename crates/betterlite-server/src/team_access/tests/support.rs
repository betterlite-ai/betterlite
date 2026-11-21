use std::io::Error as IoError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_auth::auth_user_lookup::rows::UserCreateData;
use betterlite_auth::auth_user_lookup::store::{UserStore, UserStoreError, UserStoreResult};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::TeamBase;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio_util::task::TaskTracker;

use crate::team_access::access::OrgRoles;
use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;

pub(super) fn team(text: &str) -> TeamBase<String> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> TeamCaller {
    TeamCaller {
        user_id: user_id.map(str::to_owned),
        user_role,
        team_id: None,
        team_service_account: false,
    }
}

// answers from (user, organization) pairs and records every lookup it is asked for
#[derive(Default)]
pub(super) struct RecordingOrgRoles {
    pub(super) admins: Vec<(&'static str, &'static str)>,
    pub(super) lookups: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl OrgRoles for RecordingOrgRoles {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        let pair = (user_id.to_owned(), organization_id.to_owned());
        self.lookups.lock().unwrap().push(pair);
        Ok(self
            .admins
            .iter()
            .any(|&(user, organization)| user == user_id && organization == organization_id))
    }
}

// the cache tier, the user and team tables and the permission reads behind B736 and B737
#[derive(Default)]
pub(super) struct World {
    pub(super) cached_user: Option<LiteLLMUserTable>,
    pub(super) cached_team: Option<LiteLLMTeamTableCachedObj>,
    pub(super) team_row: Option<JsonData>,
    pub(super) user_reads: AtomicUsize,
    pub(super) user_creates: AtomicUsize,
    pub(super) team_cache_reads: AtomicUsize,
    pub(super) team_reads: Mutex<Vec<bool>>,
}

pub(super) fn users(world: &Arc<World>, store: Option<Arc<dyn UserStore>>) -> UserLookup {
    UserLookup {
        cache: Arc::<World>::clone(world),
        store,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

pub(super) fn teams(world: &Arc<World>) -> TeamLookup {
    TeamLookup {
        cache: Arc::<World>::clone(world),
        permission_cache: Arc::<World>::clone(world),
        database: Some(TeamDatabase {
            store: Arc::<World>::clone(world),
            permission_reader: Arc::<World>::clone(world),
            permission_writer: Arc::<World>::clone(world),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new(
            "authorization never computes a budget reset",
        ))
    }
}

#[async_trait]
impl UserCache for World {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self.cached_user.clone())
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

#[async_trait]
impl UserStore for World {
    async fn find_by_id(
        &self,
        _user_id: &str,
        _use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        self.user_reads.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }

    async fn find_by_sso_id(&self, _sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        Ok(None)
    }

    async fn find_by_email(&self, _email: &str) -> UserStoreResult<Option<JsonData>> {
        Ok(None)
    }

    async fn create(&self, _data: UserCreateData) -> UserStoreResult<JsonData> {
        self.user_creates.fetch_add(1, Ordering::SeqCst);
        Err(UserStoreError::Other(Box::new(IoError::other(
            "no user is created during authorization",
        ))))
    }

    async fn update_sso_user_id(&self, _user_id: &str, _sso_user_id: &str) -> UserStoreResult<()> {
        Ok(())
    }

    async fn backfill_null_email(&self, _user_id: &str, _email: &str) -> UserStoreResult<()> {
        Ok(())
    }

    async fn reread_for_backfill(
        &self,
        _user_id: &str,
    ) -> UserStoreResult<Option<LiteLLMUserTable>> {
        Ok(None)
    }
}

#[async_trait]
impl TeamCache for World {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.team_cache_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.cached_team.clone())
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
    async fn find_team(&self, _team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.team_reads.lock().unwrap().push(use_writer);
        Ok(self.team_row.clone())
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
