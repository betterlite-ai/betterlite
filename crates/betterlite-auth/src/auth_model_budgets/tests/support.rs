use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio_util::task::TaskTracker;

use crate::auth_user_lookup::cache::{UserCache, UserCacheError};
use crate::auth_user_lookup::cadence::{Cadence, CadenceClock};
use crate::auth_user_lookup::lookup::UserLookup;
use crate::auth_user_lookup::ports::{BudgetResetCalculator, DefaultTeamEnrollment, PortError};
use crate::auth_user_lookup::rows::UserCreateData;
use crate::auth_user_lookup::store::{UserStore, UserStoreError, UserStoreResult};
use crate::resolved_key::auth::UserAPIKeyAuth;

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) row: Mutex<Option<JsonData>>,
    pub(crate) finds: Mutex<Vec<String>>,
    pub(crate) creates: Mutex<Vec<BTreeMap<String, JsonData>>>,
    pub(crate) fail: bool,
}

#[async_trait]
impl UserStore for FakeStore {
    async fn find_by_id(
        &self,
        user_id: &str,
        _use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        self.finds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(user_id.to_owned());
        if self.fail {
            return Err(UserStoreError::Other(Box::new(IoError::other(
                "the user read failed",
            ))));
        }
        Ok(self
            .row
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn find_by_sso_id(&self, _sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        Ok(None)
    }

    async fn find_by_email(&self, _email: &str) -> UserStoreResult<Option<JsonData>> {
        Ok(None)
    }

    async fn create(&self, data: UserCreateData) -> UserStoreResult<JsonData> {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.fields.clone());
        Ok(JsonData::Object(data.fields))
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

#[derive(Default)]
pub(crate) struct FakeCache {
    pub(crate) cached: Mutex<Option<LiteLLMUserTable>>,
    pub(crate) writes: Mutex<Vec<String>>,
}

#[async_trait]
impl UserCache for FakeCache {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn write(
        &self,
        key: &str,
        user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(user.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

pub(crate) struct StaticClock;

impl CadenceClock for StaticClock {
    fn now(&self) -> Duration {
        Duration::from_secs(0)
    }
}

pub(crate) struct FakeReset;

impl BudgetResetCalculator for FakeReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        serde_json::from_str("\"2026-01-01T00:00:00+00:00\"")
            .map_err(|_source| PortError::new("the fixture timestamp is RFC 3339"))
    }
}

pub(crate) struct FakeEnrollment;

#[async_trait]
impl DefaultTeamEnrollment for FakeEnrollment {
    async fn enroll(
        &self,
        _user_id: &str,
        _user_email: Option<&str>,
        _auth: &UserAPIKeyAuth,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

pub(crate) struct Harness {
    pub(crate) store: Arc<FakeStore>,
    pub(crate) cache: Arc<FakeCache>,
    pub(crate) with_store: bool,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            store: Arc::new(FakeStore::default()),
            cache: Arc::new(FakeCache::default()),
            with_store: true,
        }
    }
}

impl Harness {
    pub(crate) fn lookup(&self) -> UserLookup {
        let cache: Arc<dyn UserCache> = Arc::<FakeCache>::clone(&self.cache);
        let store: Arc<dyn UserStore> = Arc::<FakeStore>::clone(&self.store);
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the patterns compile");
        UserLookup {
            cache,
            store: self.with_store.then_some(store),
            cadence: Arc::new(Cadence::new(100, Arc::new(StaticClock))),
            db_cache_expiry: 60,
            budget_reset: Arc::new(FakeReset),
            enrollment: Arc::new(FakeEnrollment),
            tasks: TaskTracker::new(),
            defaults: None,
            redactor: Arc::new(redactor),
        }
    }
}
