#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use tokio_util::task::TaskTracker;

use crate::auth_user_lookup::cache::{UserCache, UserCacheError};
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookup;
use crate::auth_user_lookup::ports::{BudgetResetCalculator, DefaultTeamEnrollment, PortError};
use crate::auth_user_lookup::rows::UserCreateData;
use crate::auth_user_lookup::store::{UserStore, UserStoreError, UserStoreResult};
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::ui_session_identity::UiSessionTeams as _;

#[derive(Default)]
struct FakeStore {
    row: Mutex<Option<JsonData>>,
    failure: Mutex<Option<UserStoreError>>,
    reads: Mutex<Vec<(String, bool)>>,
    other: Mutex<Vec<&'static str>>,
}

impl FakeStore {
    fn reads(&self) -> Vec<(String, bool)> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn other_calls(&self) -> Vec<&'static str> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UserStore for FakeStore {
    async fn find_by_id(
        &self,
        user_id: &str,
        use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), use_writer));
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(self
            .row
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn find_by_sso_id(&self, _sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("find_by_sso_id");
        Ok(None)
    }

    async fn find_by_email(&self, _email: &str) -> UserStoreResult<Option<JsonData>> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("find_by_email");
        Ok(None)
    }

    async fn create(&self, _data: UserCreateData) -> UserStoreResult<JsonData> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("create");
        Err(UserStoreError::Other(Box::new(IoError::other(
            "the cases never create a user",
        ))))
    }

    async fn update_sso_user_id(&self, _user_id: &str, _sso_user_id: &str) -> UserStoreResult<()> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("update_sso_user_id");
        Ok(())
    }

    async fn backfill_null_email(&self, _user_id: &str, _email: &str) -> UserStoreResult<()> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("backfill_null_email");
        Ok(())
    }

    async fn reread_for_backfill(
        &self,
        _user_id: &str,
    ) -> UserStoreResult<Option<LiteLLMUserTable>> {
        self.other
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("reread_for_backfill");
        Ok(None)
    }
}

struct FakeCache {
    cached: Mutex<Option<LiteLLMUserTable>>,
    reads: Mutex<Vec<String>>,
}

impl FakeCache {
    fn new() -> Self {
        Self {
            cached: Mutex::new(None),
            reads: Mutex::new(Vec::new()),
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn cached_teams(&self) -> Option<Vec<String>> {
        self.cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|user| user.teams.clone())
    }
}

#[async_trait]
impl UserCache for FakeCache {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        Ok(self
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn write(
        &self,
        _key: &str,
        user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(user.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct NoBudgetReset;

impl BudgetResetCalculator for NoBudgetReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("the cases never create a user"))
    }
}

struct NoEnrollment;

#[async_trait]
impl DefaultTeamEnrollment for NoEnrollment {
    async fn enroll(
        &self,
        _user_id: &str,
        _user_email: Option<&str>,
        _auth: &UserAPIKeyAuth,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

struct Case {
    store: Arc<FakeStore>,
    cache: Arc<FakeCache>,
    lookup: UserLookup,
}

fn case(with_store: bool) -> Case {
    let store = Arc::new(FakeStore::default());
    let cache = Arc::new(FakeCache::new());
    let store_handle: Arc<FakeStore> = Arc::clone(&store);
    let reader: Arc<dyn UserStore> = store_handle;
    let cache_handle: Arc<FakeCache> = Arc::clone(&cache);
    let cached: Arc<dyn UserCache> = cache_handle;
    let lookup = UserLookup {
        cache: cached,
        store: with_store.then_some(reader),
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoBudgetReset),
        enrollment: Arc::new(NoEnrollment),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    };
    Case {
        store,
        cache,
        lookup,
    }
}

fn row(user_id: &str, teams: &[&str]) -> JsonData {
    let team_ids = teams
        .iter()
        .map(|team_id| JsonData::String((*team_id).to_owned()))
        .collect();
    JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        ("teams".to_owned(), JsonData::Array(team_ids)),
    ]))
}

fn model(record: &JsonData) -> LiteLLMUserTable {
    user_record_to_model(Some(record)).unwrap().unwrap()
}

#[rstest]
#[case::with_a_store(true)]
#[case::without_a_store(false)]
fn has_database_reports_whether_the_store_is_set(#[case] with_store: bool) {
    let case = case(with_store);
    assert_eq!(case.lookup.has_database(), with_store);
}

#[tokio::test]
async fn a_fresh_read_skips_the_cache_and_asks_the_writer() {
    let case = case(true);
    *case.store.row.lock().unwrap() = Some(row("user-1", &["team-1"]));
    let teams = case.lookup.user_teams("user-1", true).await.unwrap();
    assert_eq!(teams, Some(vec!["team-1".to_owned()]));
    assert!(case.cache.reads().is_empty());
    assert_eq!(case.store.reads(), vec![("user-1".to_owned(), true)]);
}

#[tokio::test]
async fn a_cached_read_checks_the_cache_first_then_the_reader() {
    let case = case(true);
    *case.store.row.lock().unwrap() = Some(row("user-1", &["team-1"]));
    let teams = case.lookup.user_teams("user-1", false).await.unwrap();
    assert_eq!(teams, Some(vec!["team-1".to_owned()]));
    assert_eq!(case.cache.reads(), vec!["user-1".to_owned()]);
    assert_eq!(case.store.reads(), vec![("user-1".to_owned(), false)]);
}

#[tokio::test]
async fn the_cached_user_is_answered_without_a_database_read() {
    let case = case(true);
    *case.cache.cached.lock().unwrap() = Some(model(&row("user-1", &["team-1"])));
    let teams = case.lookup.user_teams("user-1", false).await.unwrap();
    assert_eq!(teams, Some(vec!["team-1".to_owned()]));
    assert!(case.store.reads().is_empty());
}

#[tokio::test]
async fn the_user_row_is_mapped_to_its_teams() {
    let case = case(true);
    *case.store.row.lock().unwrap() = Some(row("user-1", &["team-2", "litellm-dashboard"]));
    let teams = case.lookup.user_teams("user-1", true).await.unwrap();
    assert_eq!(
        teams,
        Some(vec!["team-2".to_owned(), "litellm-dashboard".to_owned()])
    );
    assert_eq!(
        case.cache.cached_teams(),
        Some(vec!["team-2".to_owned(), "litellm-dashboard".to_owned()])
    );
    assert!(case.store.other_calls().is_empty());
}

#[tokio::test]
async fn the_missing_user_is_an_error_rather_than_no_user() {
    let case = case(true);
    let result = case.lookup.user_teams("user-1", true).await;
    assert!(matches!(result, Err(UserLookupError::NotFound(_))));
    assert!(case.store.other_calls().is_empty());
}

#[tokio::test]
async fn a_lookup_error_comes_back_with_its_cause() {
    let case = case(true);
    *case.store.failure.lock().unwrap() = Some(UserStoreError::Other(Box::new(IoError::other(
        "the store is down",
    ))));
    let result = case.lookup.user_teams("user-1", true).await;
    let error = result.err().unwrap();
    assert!(
        matches!(error, UserLookupError::Unavailable(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("the store is down"), "{error}");
}

#[tokio::test]
async fn a_read_without_a_store_reports_no_database() {
    let case = case(false);
    let result = case.lookup.user_teams("user-1", true).await;
    assert!(matches!(result, Err(UserLookupError::NoDatabase)));
}
