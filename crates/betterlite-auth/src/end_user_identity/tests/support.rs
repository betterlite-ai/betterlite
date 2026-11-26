#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{
    DualCache, DualCacheError, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
};
use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable, Timestamp};
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry as tracing_registry;

use super::super::cache::{VALIDATION_KEY_PREFIX, ValidationCache};
use super::super::existence::{EndUserExistence, ExistenceError, UserExistence, UserProbe};
use super::super::resolve::{DatabaseLookups, EndUserIdResolver, ResolveRequest};
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteTtl, gateway_cache};
use crate::auth_end_users::cache::EndUserCache;
use crate::auth_end_users::object::EndUserObjectError;
use crate::auth_end_users::rows::{BudgetFetchError, BudgetRows, EndUserFetchError, EndUserRows};
use crate::auth_read::table::AuthReadError;
use crate::auth_user_lookup::cache::{UserCache, UserCacheError};
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookup;
use crate::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use crate::auth_user_lookup::rows::UserCreateData;
use crate::auth_user_lookup::store::{UserStore, UserStoreResult};
use crate::end_user_registry::EndUserRegistry;
use crate::end_user_registry::cache::RegistryCache;

#[derive(Clone, Debug)]
pub(super) struct WriteRecord {
    pub(super) cache_key: String,
    pub(super) value: Json,
    pub(super) ttl: WriteTtl,
}

#[derive(Debug)]
pub(super) struct ScriptedCache {
    pub(super) values: Mutex<BTreeMap<String, Json>>,
    pub(super) reads: Mutex<Vec<String>>,
    pub(super) writes: Mutex<Vec<WriteRecord>>,
    pub(super) fail_reads: bool,
    pub(super) fail_writes: bool,
}

// which of the two cache calls fails, in place of the two bools the build cannot take
#[derive(Clone, Copy, Debug)]
enum Failures {
    None,
    Reads,
    Writes,
}

impl ScriptedCache {
    pub(super) fn new() -> Self {
        Self::build(Failures::None)
    }

    pub(super) fn failing_reads() -> Self {
        Self::build(Failures::Reads)
    }

    pub(super) fn failing_writes() -> Self {
        Self::build(Failures::Writes)
    }

    fn build(failure: Failures) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            fail_reads: matches!(failure, Failures::Reads),
            fail_writes: matches!(failure, Failures::Writes),
        }
    }

    fn record_read(&self, cache_key: &str) -> Result<(), AuthCacheError> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(cache_key.to_owned());
        if self.fail_reads {
            return Err(cache_failure());
        }
        Ok(())
    }

    pub(super) fn seed(&self, end_user_id: &str, value: Json) {
        self.values
            .lock()
            .expect("the cache contents")
            .insert(format!("{VALIDATION_KEY_PREFIX}{end_user_id}"), value);
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads").clone()
    }

    pub(super) fn writes(&self) -> Vec<WriteRecord> {
        self.writes.lock().expect("the cache writes").clone()
    }
}

pub(super) fn cache_failure() -> AuthCacheError {
    AuthCacheError::from(DualCacheError::Memory(BackendError::Unsupported(
        "the validation cache",
    )))
}

#[async_trait]
impl ValidationCache for ScriptedCache {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.record_read(cache_key)?;
        let values = self.values.lock().expect("the cache contents");
        Ok(values.get(cache_key).cloned())
    }

    async fn write(
        &self,
        cache_key: &str,
        value: &Json,
        ttl: WriteTtl,
    ) -> Result<(), AuthCacheError> {
        self.writes
            .lock()
            .expect("the cache writes")
            .push(WriteRecord {
                cache_key: cache_key.to_owned(),
                value: value.clone(),
                ttl,
            });
        if self.fail_writes {
            return Err(cache_failure());
        }
        self.values
            .lock()
            .expect("the cache contents")
            .insert(cache_key.to_owned(), value.clone());
        Ok(())
    }
}

pub(super) fn assert_wrote(
    cache: &ScriptedCache,
    end_user_id: &str,
    verdict: &str,
    ttl_seconds: f64,
) {
    let writes = cache.writes();
    assert_eq!(writes.len(), 1);
    let write = &writes[0];
    assert_eq!(
        write.cache_key,
        format!("{VALIDATION_KEY_PREFIX}{end_user_id}")
    );
    assert_eq!(write.value, Json::Text(verdict.to_owned()));
    assert_eq!(
        write.ttl,
        WriteTtl {
            seconds: ttl_seconds
        }
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    Found,
    Missing,
    Error,
}

impl Answer {
    fn found(self) -> Option<bool> {
        match self {
            Self::Found => Some(true),
            Self::Missing => Some(false),
            Self::Error => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct ScriptedEndUsers {
    pub(super) answer: Answer,
    pub(super) calls: Mutex<Vec<String>>,
}

impl ScriptedEndUsers {
    pub(super) fn new(answer: Answer) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the end-user calls").clone()
    }
}

#[async_trait]
impl EndUserExistence for ScriptedEndUsers {
    async fn end_user_exists(&self, end_user_id: &str) -> Result<bool, ExistenceError> {
        self.calls
            .lock()
            .expect("the end-user calls")
            .push(end_user_id.to_owned());
        let found = self.answer.found();
        found.ok_or(ExistenceError::EndUser(EndUserObjectError::NoDatabase))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProbeRecord {
    pub(super) user_id: String,
    pub(super) user_email: Option<String>,
    pub(super) user_id_upsert: bool,
    pub(super) check_db_only: bool,
}

#[derive(Debug)]
pub(super) struct ScriptedUsers {
    pub(super) answer: Answer,
    pub(super) probes: Mutex<Vec<ProbeRecord>>,
}

impl ScriptedUsers {
    pub(super) fn new(answer: Answer) -> Self {
        Self {
            answer,
            probes: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn probes(&self) -> Vec<ProbeRecord> {
        self.probes.lock().expect("the user probes").clone()
    }
}

#[async_trait]
impl UserExistence for ScriptedUsers {
    async fn user_exists(&self, probe: &UserProbe<'_>) -> Result<bool, ExistenceError> {
        self.probes
            .lock()
            .expect("the user probes")
            .push(ProbeRecord {
                user_id: probe.user_id.to_owned(),
                user_email: probe.user_email.map(str::to_owned),
                user_id_upsert: probe.user_id_upsert,
                check_db_only: probe.check_db_only,
            });
        let found = self.answer.found();
        found.ok_or(ExistenceError::User(UserLookupError::NoDatabase))
    }
}

pub(super) fn lookups(
    end_users: &Arc<ScriptedEndUsers>,
    users: &Arc<ScriptedUsers>,
) -> DatabaseLookups {
    DatabaseLookups {
        end_users: Arc::<ScriptedEndUsers>::clone(end_users),
        users: Arc::<ScriptedUsers>::clone(users),
    }
}

pub(super) const fn request(end_user_id: &str) -> ResolveRequest<'_> {
    ResolveRequest {
        raw_end_user_id: Some(end_user_id),
        key_end_user_budget_id: None,
    }
}

pub(super) fn resolver_for(
    cache: Arc<ScriptedCache>,
    end_users: &Arc<ScriptedEndUsers>,
    users: &Arc<ScriptedUsers>,
    validate: bool,
    max_end_user_budget_id: Option<&str>,
) -> EndUserIdResolver {
    EndUserIdResolver {
        validate_end_user_id_in_db: validate,
        max_end_user_budget_id: max_end_user_budget_id.map(str::to_owned),
        lookups: Some(lookups(end_users, users)),
        cache,
    }
}

pub(super) fn end_user_row(user_id: &str) -> LiteLLMEndUserTable {
    LiteLLMEndUserTable {
        user_id: user_id.to_owned(),
        blocked: PydanticBool(false),
        alias: None,
        spend: Decimal::ZERO,
        allowed_model_region: None,
        default_model: None,
        budget_id: None,
        litellm_budget_table: None,
        object_permission_id: None,
        object_permission: None,
    }
}

// the management cache B739 reads, with rows and payloads that may not decode
#[derive(Debug)]
pub(super) struct RowCache {
    pub(super) values: Mutex<BTreeMap<String, Json>>,
    pub(super) reads: Mutex<Vec<String>>,
}

impl RowCache {
    pub(super) fn new() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seed_row<Row>(&self, cache_key: &str, row: &Row)
    where
        Row: Serialize,
    {
        let value = serde_json::to_value(row)
            .and_then(serde_json::from_value::<Json>)
            .expect("the seeded row serializes");
        self.seed_json(cache_key, value);
    }

    pub(super) fn seed_json(&self, cache_key: &str, value: Json) {
        self.values
            .lock()
            .expect("the cache contents")
            .insert(cache_key.to_owned(), value);
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads").clone()
    }
}

#[async_trait]
impl EndUserCache for RowCache {
    async fn read_row<Row>(&self, cache_key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned + Send + 'static,
    {
        self.reads
            .lock()
            .expect("the cache reads")
            .push(cache_key.to_owned());
        let stored = self
            .values
            .lock()
            .expect("the cache contents")
            .get(cache_key)
            .cloned();
        stored.map_or(Ok(None), |cached| {
            serde_json::to_value(cached)
                .map_err(PayloadError::Json)
                .and_then(|value| serde_json::from_value::<Row>(value).map_err(PayloadError::Json))
                .map(Some)
        })
    }

    async fn write_row<Row>(
        &self,
        cache_key: &str,
        row: &Row,
        _ttl: f64,
    ) -> Result<(), AuthCacheError>
    where
        Row: Serialize + Sync,
    {
        let value = serde_json::to_value(row).map_err(PayloadError::Json)?;
        let encoded = serde_json::from_value::<Json>(value).map_err(PayloadError::Json)?;
        self.values
            .lock()
            .expect("the cache contents")
            .insert(cache_key.to_owned(), encoded);
        Ok(())
    }
}

#[async_trait]
impl RegistryCache for RowCache {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.reads
            .lock()
            .expect("the cache reads")
            .push(cache_key.to_owned());
        Ok(None)
    }

    async fn write(&self, _key: &str, _answer: &Json, _ttl: f64) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        45.0_f64
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RowAnswer {
    Missing,
    Failure,
}

// the end-user and budget rows B739 reads, with a seeded row winning over the fallback
#[derive(Debug)]
pub(super) struct ScriptedRows {
    pub(super) end_users: Mutex<BTreeMap<String, LiteLLMEndUserTable>>,
    pub(super) end_user_answer: RowAnswer,
    pub(super) budget_answer: RowAnswer,
    pub(super) queries: Mutex<Vec<String>>,
}

impl ScriptedRows {
    pub(super) fn new(end_user_answer: RowAnswer, budget_answer: RowAnswer) -> Self {
        Self {
            end_users: Mutex::new(BTreeMap::new()),
            end_user_answer,
            budget_answer,
            queries: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seed(&self, row: LiteLLMEndUserTable) {
        self.end_users
            .lock()
            .expect("the seeded end users")
            .insert(row.user_id.clone(), row);
    }

    pub(super) fn queries(&self) -> Vec<String> {
        self.queries.lock().expect("the row queries").clone()
    }
}

fn row_failure<Failure>() -> AuthReadError<Failure>
where
    Failure: Error + 'static,
{
    AuthReadError::Repository(DbError::NoDbConnected)
}

#[async_trait]
impl EndUserRows for ScriptedRows {
    async fn find_end_user(
        &self,
        end_user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFetchError> {
        self.queries
            .lock()
            .expect("the row queries")
            .push(end_user_id.to_owned());
        let seeded = self
            .end_users
            .lock()
            .expect("the seeded end users")
            .get(end_user_id)
            .cloned();
        if let Some(row) = seeded {
            return Ok(Some(row));
        }
        match self.end_user_answer {
            RowAnswer::Missing => Ok(None),
            RowAnswer::Failure => Err(EndUserFetchError::Read(Box::new(row_failure()))),
        }
    }
}

#[async_trait]
impl BudgetRows for ScriptedRows {
    async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetFetchError> {
        self.queries
            .lock()
            .expect("the row queries")
            .push(budget_id.to_owned());
        match self.budget_answer {
            RowAnswer::Missing => Ok(None),
            RowAnswer::Failure => Err(BudgetFetchError::Query(Box::new(row_failure()))),
        }
    }
}

pub(super) fn registry(validate: bool, max_end_user_budget_id: Option<&str>) -> EndUserRegistry {
    EndUserRegistry::new(Settings {
        validate_end_user_id_in_db: validate,
        max_end_user_budget_id: max_end_user_budget_id.map(str::to_owned),
    })
}

// the B736 ports: a store that records its calls and a cache that holds one user row
#[derive(Default)]
pub(super) struct ScriptedUserStore {
    pub(super) by_email: Mutex<Option<JsonData>>,
    pub(super) calls: Mutex<Vec<&'static str>>,
}

impl ScriptedUserStore {
    pub(super) fn answer_email(&self, user_id: &str, email: &str) {
        let mut fields =
            BTreeMap::from([("user_id".to_owned(), JsonData::String(user_id.to_owned()))]);
        fields.insert("user_email".to_owned(), JsonData::String(email.to_owned()));
        *self.by_email.lock().expect("the email answer") = Some(JsonData::Object(fields));
    }

    pub(super) fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("the store calls").clone()
    }
}

#[async_trait]
impl UserStore for ScriptedUserStore {
    async fn find_by_id(
        &self,
        _user_id: &str,
        _use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("find_by_id");
        Ok(None)
    }

    async fn find_by_sso_id(&self, _sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("find_by_sso_id");
        Ok(None)
    }

    async fn find_by_email(&self, _email: &str) -> UserStoreResult<Option<JsonData>> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("find_by_email");
        Ok(self.by_email.lock().expect("the email answer").clone())
    }

    async fn create(&self, data: UserCreateData) -> UserStoreResult<JsonData> {
        self.calls.lock().expect("the store calls").push("create");
        Ok(JsonData::Object(data.fields))
    }

    async fn update_sso_user_id(&self, _user_id: &str, _sso_user_id: &str) -> UserStoreResult<()> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("update_sso_user_id");
        Ok(())
    }

    async fn backfill_null_email(&self, _user_id: &str, _email: &str) -> UserStoreResult<()> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("backfill_null_email");
        Ok(())
    }

    async fn reread_for_backfill(
        &self,
        _user_id: &str,
    ) -> UserStoreResult<Option<LiteLLMUserTable>> {
        self.calls
            .lock()
            .expect("the store calls")
            .push("reread_for_backfill");
        Ok(None)
    }
}

#[derive(Default)]
pub(super) struct UserCachePort {
    pub(super) cached: Mutex<Option<LiteLLMUserTable>>,
    pub(super) reads: Mutex<Vec<String>>,
    pub(super) writes: Mutex<Vec<String>>,
}

impl UserCachePort {
    pub(super) fn write_keys(&self) -> Vec<String> {
        self.writes.lock().expect("the user cache writes").clone()
    }
}

#[async_trait]
impl UserCache for UserCachePort {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.reads
            .lock()
            .expect("the user cache reads")
            .push(key.to_owned());
        Ok(self.cached.lock().expect("the cached user").clone())
    }

    async fn write(
        &self,
        key: &str,
        user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        self.writes
            .lock()
            .expect("the user cache writes")
            .push(key.to_owned());
        *self.cached.lock().expect("the cached user") = Some(user.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        45.0_f64
    }
}

struct NoBudgetReset;

impl BudgetResetCalculator for NoBudgetReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("a validation probe never creates a user"))
    }
}

pub(super) fn user_lookup(
    cache: &Arc<UserCachePort>,
    store: &Arc<ScriptedUserStore>,
) -> UserLookup {
    lookup_over(
        Arc::<UserCachePort>::clone(cache),
        Arc::<ScriptedUserStore>::clone(store),
    )
}

pub(super) fn lookup_over(cache: Arc<dyn UserCache>, store: Arc<dyn UserStore>) -> UserLookup {
    let reset: Arc<dyn BudgetResetCalculator> = Arc::new(NoBudgetReset);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
    UserLookup {
        cache,
        store: Some(store),
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: reset,
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(redactor),
    }
}

pub(super) type TestCache = AuthCache<DualCache<LocalMemoryOps>>;

fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

fn local_memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"))
}

// the production cache over local memory, for the Postgres lookup
pub(super) fn test_cache() -> TestCache {
    gateway_cache(
        local_memory(),
        local_memory(),
        None,
        dual_settings(),
        45.0_f64,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LoggedEvent {
    pub(super) level: Level,
    pub(super) message: String,
}

#[derive(Default)]
struct EventFields {
    message: String,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct EventLog(Arc<Mutex<Vec<LoggedEvent>>>);

impl<S: Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0
            .lock()
            .expect("the captured events")
            .push(LoggedEvent {
                level: *event.metadata().level(),
                message: fields.message,
            });
    }
}

pub(super) fn capture_events() -> (EventLog, DefaultGuard) {
    let log = EventLog::default();
    let guard = set_default(tracing_registry().with(log.clone()));
    (log, guard)
}

pub(super) fn debug_messages(log: &EventLog) -> Vec<String> {
    log.0
        .lock()
        .expect("the captured events")
        .iter()
        .filter(|event| event.level == Level::DEBUG)
        .map(|event| event.message.clone())
        .collect()
}
