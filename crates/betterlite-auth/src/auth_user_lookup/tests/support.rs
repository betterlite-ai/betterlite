use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::future::pending;
use std::io::Error as IoError;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::auth_user_lookup::DefaultUserParams;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_user_lookup::cache::{UserCache, UserCacheError};
use crate::auth_user_lookup::cadence::{Cadence, CadenceClock, CadenceEntry};
use crate::auth_user_lookup::error::{UserLookupError, user_read_failure};
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::auth_user_lookup::ports::{BudgetResetCalculator, DefaultTeamEnrollment, PortError};
use crate::auth_user_lookup::rows::UserCreateData;
use crate::auth_user_lookup::store::{UserStore, UserStoreError, UserStoreResult};
use crate::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct NamedFailure(pub(crate) String);

#[derive(Clone, Debug)]
pub(crate) enum FailureKind {
    Named(String),
    Io(String),
}

fn failure_error(kind: &FailureKind) -> UserStoreError {
    match kind {
        FailureKind::Named(message) => {
            UserStoreError::Other(Box::new(NamedFailure(message.clone())))
        }
        FailureKind::Io(message) => {
            UserStoreError::Other(Box::new(IoError::other(message.clone())))
        }
    }
}

pub(crate) fn unavailable(cause: Box<dyn Error + Send + Sync>) -> bool {
    match user_read_failure("u1", cause) {
        UserLookupError::Unavailable(_) => true,
        UserLookupError::UserMissing(_) => false,
        other => panic!("a read failure classifies as unavailable or missing: {other:?}"),
    }
}

pub(crate) type Events = Arc<Mutex<Vec<String>>>;

pub(crate) fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

#[derive(Debug, Default)]
pub(crate) struct ManualClock {
    millis: AtomicU64,
}

impl ManualClock {
    pub(crate) fn set(&self, at: Duration) {
        let millis = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
        self.millis.store(millis, Ordering::SeqCst);
    }
}

impl CadenceClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
}

pub(crate) struct ScriptedClock {
    times: Mutex<VecDeque<Duration>>,
    fallback: Duration,
}

impl ScriptedClock {
    pub(crate) fn new<T>(times: T, fallback: Duration) -> Self
    where
        T: IntoIterator<Item = Duration>,
    {
        Self {
            times: Mutex::new(times.into_iter().collect()),
            fallback,
        }
    }
}

impl CadenceClock for ScriptedClock {
    fn now(&self) -> Duration {
        self.times
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    FindById {
        user_id: String,
        use_writer: bool,
    },
    FindBySso {
        sso_user_id: String,
    },
    FindByEmail {
        email: String,
    },
    Create {
        fields: BTreeMap<String, JsonData>,
    },
    UpdateSso {
        user_id: String,
        sso_user_id: String,
    },
    Backfill {
        user_id: String,
        email: String,
    },
    Reread {
        user_id: String,
    },
}

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) calls: Mutex<Vec<Call>>,
    pub(crate) events: Events,
    pub(crate) by_id: Mutex<Option<JsonData>>,
    pub(crate) by_sso: Mutex<Option<JsonData>>,
    pub(crate) by_email: Mutex<Option<JsonData>>,
    pub(crate) reread: Mutex<Option<LiteLLMUserTable>>,
    pub(crate) failures: Mutex<BTreeMap<&'static str, FailureKind>>,
    pub(crate) hang: AtomicBool,
    pub(crate) block_update: AtomicBool,
    pub(crate) update_started: Notify,
    pub(crate) update_release: Notify,
}

impl Call {
    fn name(&self) -> &'static str {
        match self {
            Self::FindById { .. } => "find_by_id",
            Self::FindBySso { .. } => "find_by_sso_id",
            Self::FindByEmail { .. } => "find_by_email",
            Self::Create { .. } => "create",
            Self::UpdateSso { .. } => "update_sso",
            Self::Backfill { .. } => "backfill",
            Self::Reread { .. } => "reread",
        }
    }
}

impl FakeStore {
    pub(crate) fn methods(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap();
        calls.iter().map(Call::name).map(str::to_owned).collect()
    }

    pub(crate) fn create_fields(&self) -> Option<BTreeMap<String, JsonData>> {
        let calls = self.calls.lock().unwrap();
        calls.iter().find_map(|call| match call {
            Call::Create { fields } => Some(fields.clone()),
            Call::FindById { .. }
            | Call::FindBySso { .. }
            | Call::FindByEmail { .. }
            | Call::UpdateSso { .. }
            | Call::Backfill { .. }
            | Call::Reread { .. } => None,
        })
    }

    fn observe(&self, method: &'static str, call: Call) -> Option<UserStoreError> {
        self.calls.lock().unwrap().push(call);
        self.events.lock().unwrap().push(method.to_owned());
        let failure = self.failures.lock().unwrap().remove(method);
        failure.as_ref().map(failure_error)
    }
}

fn answer<T>(
    failed: Option<UserStoreError>,
    stored: &Mutex<Option<T>>,
) -> UserStoreResult<Option<T>>
where
    T: Clone,
{
    if let Some(error) = failed {
        return Err(error);
    }
    Ok(stored
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone())
}

#[async_trait]
impl UserStore for FakeStore {
    async fn find_by_id(
        &self,
        user_id: &str,
        use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        let call = Call::FindById {
            user_id: user_id.to_owned(),
            use_writer,
        };
        let failed = self.observe("find_by_id", call);
        if self.hang.load(Ordering::SeqCst) {
            pending::<()>().await;
        }
        answer(failed, &self.by_id)
    }

    async fn find_by_sso_id(&self, sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        let call = Call::FindBySso {
            sso_user_id: sso_user_id.to_owned(),
        };
        let failed = self.observe("find_by_sso_id", call);
        answer(failed, &self.by_sso)
    }

    async fn find_by_email(&self, email: &str) -> UserStoreResult<Option<JsonData>> {
        let call = Call::FindByEmail {
            email: email.to_owned(),
        };
        let failed = self.observe("find_by_email", call);
        answer(failed, &self.by_email)
    }

    async fn create(&self, data: UserCreateData) -> Result<JsonData, UserStoreError> {
        let mut row = data.fields.clone();
        row.entry("organization_memberships".to_owned())
            .or_insert_with(|| JsonData::Array(Vec::new()));
        let failed = self.observe(
            "create",
            Call::Create {
                fields: data.fields,
            },
        );
        if let Some(error) = failed {
            return Err(error);
        }
        Ok(JsonData::Object(row))
    }

    async fn update_sso_user_id(
        &self,
        user_id: &str,
        sso_user_id: &str,
    ) -> Result<(), UserStoreError> {
        let failed = self.observe(
            "update_sso",
            Call::UpdateSso {
                user_id: user_id.to_owned(),
                sso_user_id: sso_user_id.to_owned(),
            },
        );
        if self.block_update.load(Ordering::SeqCst) {
            self.update_started.notify_one();
            self.update_release.notified().await;
        }
        failed.map_or_else(|| Ok(()), Err)
    }

    async fn backfill_null_email(&self, user_id: &str, email: &str) -> Result<(), UserStoreError> {
        let failed = self.observe(
            "backfill",
            Call::Backfill {
                user_id: user_id.to_owned(),
                email: email.to_owned(),
            },
        );
        failed.map_or_else(|| Ok(()), Err)
    }

    async fn reread_for_backfill(
        &self,
        user_id: &str,
    ) -> UserStoreResult<Option<LiteLLMUserTable>> {
        let failed = self.observe(
            "reread",
            Call::Reread {
                user_id: user_id.to_owned(),
            },
        );
        answer(failed, &self.reread)
    }
}

pub(crate) struct FakeCache {
    pub(crate) reads: Mutex<Vec<String>>,
    pub(crate) writes: Mutex<Vec<(String, LiteLLMUserTable, f64)>>,
    pub(crate) cached: Mutex<Option<LiteLLMUserTable>>,
    pub(crate) read_fails: AtomicBool,
    pub(crate) write_fails: AtomicBool,
    pub(crate) ttl: f64,
}

impl FakeCache {
    pub(crate) fn new(ttl: f64) -> Self {
        Self {
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            cached: Mutex::new(None),
            read_fails: AtomicBool::new(false),
            write_fails: AtomicBool::new(false),
            ttl,
        }
    }

    pub(crate) fn write_keys(&self) -> Vec<String> {
        let writes = self.writes.lock().unwrap();
        writes.iter().map(|(key, _, _)| key.clone()).collect()
    }
}

fn payload_failure() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not-a-number").unwrap_err())
}

#[async_trait]
impl UserCache for FakeCache {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.reads.lock().unwrap().push(key.to_owned());
        if self.read_fails.load(Ordering::SeqCst) {
            return Err(UserCacheError::Read(payload_failure()));
        }
        Ok(self.cached.lock().unwrap().clone())
    }

    async fn write(
        &self,
        key: &str,
        user: &LiteLLMUserTable,
        ttl: f64,
    ) -> Result<(), UserCacheError> {
        self.writes
            .lock()
            .unwrap()
            .push((key.to_owned(), user.clone(), ttl));
        if self.write_fails.load(Ordering::SeqCst) {
            return Err(UserCacheError::Write(AuthCacheError::from(
                payload_failure(),
            )));
        }
        *self.cached.lock().unwrap() = Some(user.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        self.ttl
    }
}

pub(crate) struct FakeReset {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) events: Events,
    pub(crate) fail: AtomicBool,
    pub(crate) reset: Timestamp,
}

impl Default for FakeReset {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            events: events(),
            fail: AtomicBool::new(false),
            reset: serde_json::from_str("\"2026-01-01T00:00:00+00:00\"")
                .expect("the fixture timestamp is RFC 3339"),
        }
    }
}

impl BudgetResetCalculator for FakeReset {
    fn reset_time(&self, budget_duration: &str) -> Result<Timestamp, PortError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(budget_duration.to_owned());
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("reset".to_owned());
        if self.fail.load(Ordering::SeqCst) {
            return Err(PortError::new("budget reset failed"));
        }
        Ok(self.reset)
    }
}

pub(crate) struct FakeEnrollment {
    pub(crate) calls: Mutex<Vec<(String, Option<String>, bool)>>,
    pub(crate) events: Events,
    pub(crate) fail: AtomicBool,
}

impl Default for FakeEnrollment {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            events: events(),
            fail: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl DefaultTeamEnrollment for FakeEnrollment {
    async fn enroll(
        &self,
        user_id: &str,
        user_email: Option<&str>,
        auth: &UserAPIKeyAuth,
    ) -> Result<(), PortError> {
        self.calls.lock().unwrap().push((
            user_id.to_owned(),
            user_email.map(str::to_owned),
            user_api_key_has_admin_view(auth),
        ));
        self.events.lock().unwrap().push("enroll".to_owned());
        if self.fail.load(Ordering::SeqCst) {
            return Err(PortError::new("default team enrollment failed"));
        }
        Ok(())
    }
}

pub(crate) struct Harness {
    pub(crate) store: Arc<FakeStore>,
    pub(crate) cache: Arc<FakeCache>,
    pub(crate) clock: Arc<ManualClock>,
    pub(crate) cadence: Arc<Cadence>,
    pub(crate) reset: Arc<FakeReset>,
    pub(crate) enrollment: Arc<FakeEnrollment>,
    pub(crate) events: Events,
    pub(crate) expiry: i64,
    pub(crate) defaults: Option<DefaultUserParams>,
    pub(crate) with_store: bool,
}

impl Harness {
    pub(crate) fn new(expiry: i64) -> Self {
        let events = events();
        let clock = Arc::new(ManualClock::default());
        let cadence = Arc::new(Cadence::new(100, Arc::<ManualClock>::clone(&clock)));
        let store = Arc::new(FakeStore {
            calls: Mutex::new(Vec::new()),
            events: Arc::clone(&events),
            by_id: Mutex::new(None),
            by_sso: Mutex::new(None),
            by_email: Mutex::new(None),
            reread: Mutex::new(None),
            failures: Mutex::new(BTreeMap::new()),
            hang: AtomicBool::new(false),
            block_update: AtomicBool::new(false),
            update_started: Notify::new(),
            update_release: Notify::new(),
        });
        let reset = Arc::new(FakeReset {
            calls: Mutex::new(Vec::new()),
            events: Arc::clone(&events),
            fail: AtomicBool::new(false),
            reset: serde_json::from_str("\"2026-01-01T00:00:00+00:00\"")
                .expect("the fixture timestamp is RFC 3339"),
        });
        let enrollment = Arc::new(FakeEnrollment {
            calls: Mutex::new(Vec::new()),
            events: Arc::clone(&events),
            fail: AtomicBool::new(false),
        });
        Self {
            store,
            cache: Arc::new(FakeCache::new(60.0)),
            clock,
            cadence,
            reset,
            enrollment,
            events,
            expiry,
            defaults: None,
            with_store: true,
        }
    }

    pub(crate) fn lookup(&self) -> UserLookup {
        let cache: Arc<dyn UserCache> = Arc::<FakeCache>::clone(&self.cache);
        let handle: Arc<dyn UserStore> = Arc::<FakeStore>::clone(&self.store);
        let store = self.with_store.then_some(handle);
        let reset: Arc<dyn BudgetResetCalculator> = Arc::<FakeReset>::clone(&self.reset);
        let enrollment: Arc<dyn DefaultTeamEnrollment> =
            Arc::<FakeEnrollment>::clone(&self.enrollment);
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
        UserLookup {
            cache,
            store,
            cadence: Arc::clone(&self.cadence),
            db_cache_expiry: self.expiry,
            budget_reset: reset,
            enrollment,
            tasks: TaskTracker::new(),
            defaults: self.defaults.clone(),
            redactor: Arc::new(redactor),
        }
    }
}

impl Default for Harness {
    fn default() -> Self {
        Self::new(5)
    }
}

pub(crate) fn harness_without_store() -> Harness {
    let mut harness = Harness::new(5);
    harness.with_store = false;
    harness
}

pub(crate) fn harness_with_defaults(defaults: Option<DefaultUserParams>) -> Harness {
    let mut harness = Harness::new(5);
    harness.defaults = defaults;
    harness
}

pub(crate) fn request(user_id: Option<&str>) -> UserLookupRequest<'_> {
    request_with(user_id, None, None)
}

pub(crate) fn request_with<'request>(
    user_id: Option<&'request str>,
    sso_user_id: Option<&'request str>,
    user_email: Option<&'request str>,
) -> UserLookupRequest<'request> {
    UserLookupRequest {
        user_id,
        sso_user_id,
        user_email,
        user_id_upsert: false,
        check_db_only: None,
    }
}

pub(crate) fn cadence_entry(cadence: &Cadence, key: &str) -> Option<CadenceEntry> {
    let entries = cadence
        .entries
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    entries.entries.get(key).cloned()
}

pub(crate) fn defaults(values: &[(&str, JsonData)]) -> DefaultUserParams {
    DefaultUserParams {
        values: values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    }
}

pub(crate) fn user_row(user_id: &str) -> JsonData {
    JsonData::Object(BTreeMap::from([(
        "user_id".to_owned(),
        JsonData::String(user_id.to_owned()),
    )]))
}

pub(crate) fn user_with(user_id: &str, email: Option<&str>, sso_user_id: Option<&str>) -> JsonData {
    let mut fields = BTreeMap::from([("user_id".to_owned(), JsonData::String(user_id.to_owned()))]);
    if let Some(address) = email {
        fields.insert(
            "user_email".to_owned(),
            JsonData::String(address.to_owned()),
        );
    }
    if let Some(sso) = sso_user_id {
        fields.insert("sso_user_id".to_owned(), JsonData::String(sso.to_owned()));
    }
    JsonData::Object(fields)
}

pub(crate) fn membership(user_id: &str, organization_id: &str) -> JsonData {
    JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "organization_id".to_owned(),
            JsonData::String(organization_id.to_owned()),
        ),
        (
            "created_at".to_owned(),
            JsonData::String("2026-01-01T00:00:00+00:00".to_owned()),
        ),
        (
            "updated_at".to_owned(),
            JsonData::String("2026-01-01T00:00:00+00:00".to_owned()),
        ),
    ]))
}

pub(crate) fn model(record: &JsonData) -> LiteLLMUserTable {
    user_record_to_model(Some(record))
        .expect("the fixture row is a user row")
        .expect("the fixture row is not null")
}
