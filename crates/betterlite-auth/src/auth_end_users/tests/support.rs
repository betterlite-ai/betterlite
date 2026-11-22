#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{
    DualCache, DualCacheError, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::spend_rows::rows::{
    SpendCounterRows as DbSpendCounterRows, SpendRow, SpendRowLookup,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::cache::{
    DEFAULT_END_USER_BUDGET_PREFIX, EndUserCache, TEAM_MEMBER_BUDGET_PREFIX,
};
use super::super::rows::{BudgetFetchError, BudgetRows, EndUserFetchError, EndUserRows};
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, gateway_cache};
use crate::auth_read::table::AuthReadError;
use crate::budget_primitives::window_start::DurationParser;
use crate::end_user_registry::cache::RegistryCache;
use crate::model_budget_limiter::clock::SystemClock as UptimeClock;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reads::batch::SpendBatch;
use crate::spend_reads::cache::SpendReadCache;
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;

pub(super) type Journal = Arc<Mutex<Vec<String>>>;

pub(super) fn journal() -> Journal {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn entries<T>(shared: &Mutex<Vec<T>>) -> Vec<T>
where
    T: Clone,
{
    shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn push(shared: &Mutex<Vec<String>>, entry: String) {
    shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry);
}

pub(super) fn money(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a fixture amount")
}

pub(super) fn budget_fixture(id: &str, max_budget: Option<&str>) -> LiteLLMBudgetTable {
    let max = max_budget.map_or_else(|| "null".to_owned(), |amount| format!("\"{amount}\""));
    let text = format!(r#"{{"budget_id":"{id}","max_budget":{max}}}"#);
    serde_json::from_str(&text).expect("a budget fixture")
}

pub(super) fn end_user_row(
    id: &str,
    spend: &str,
    budget_id: Option<&str>,
    relation: Option<&LiteLLMBudgetTable>,
) -> LiteLLMEndUserTable {
    LiteLLMEndUserTable {
        user_id: id.to_owned(),
        blocked: PydanticBool(false),
        alias: None,
        spend: money(spend),
        allowed_model_region: None,
        default_model: None,
        budget_id: budget_id.map(str::to_owned),
        litellm_budget_table: relation.cloned(),
        object_permission_id: None,
        object_permission: None,
    }
}

pub(super) fn user_key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

pub(super) fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

fn cache_error() -> AuthCacheError {
    let failure = DualCacheError::Memory(BackendError::Unsupported("the end-user cache"));
    AuthCacheError::from(failure)
}

fn query_failure() -> AuthReadError<TableError> {
    AuthReadError::Lookup(DbLookupError::Database(DbError::NoDbConnected))
}

fn decode_failure() -> AuthReadError<TableError> {
    AuthReadError::Repository(DbError::InvalidRecord {
        source: TableError::Row(payload_json_error()),
    })
}

fn payload_json_error() -> serde_json::Error {
    serde_json::from_str::<i64>("not a number").expect_err("the invalid json")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Surface {
    Budget,
    EndUser,
    Registry,
}

pub(super) struct ScriptedCache {
    pub(super) registry: Mutex<Option<Json>>,
    pub(super) budgets: Mutex<BTreeMap<String, Json>>,
    pub(super) end_users: Mutex<BTreeMap<String, Json>>,
    pub(super) reads: Journal,
    pub(super) writes: Journal,
    pub(super) fail_reads: Option<Surface>,
    pub(super) fail_writes: Option<Surface>,
    pub(super) ttl: f64,
}

impl ScriptedCache {
    pub(super) fn new() -> Arc<Self> {
        Self::build(None, None, 45.0_f64)
    }

    pub(super) fn with_ttl(ttl: f64) -> Arc<Self> {
        Self::build(None, None, ttl)
    }

    pub(super) fn failing(fail_reads: Option<Surface>, fail_writes: Option<Surface>) -> Arc<Self> {
        Self::build(fail_reads, fail_writes, 45.0_f64)
    }

    fn build(fail_reads: Option<Surface>, fail_writes: Option<Surface>, ttl: f64) -> Arc<Self> {
        Arc::new(Self {
            registry: Mutex::new(None),
            budgets: Mutex::new(BTreeMap::new()),
            end_users: Mutex::new(BTreeMap::new()),
            reads: journal(),
            writes: journal(),
            fail_reads,
            fail_writes,
            ttl,
        })
    }

    pub(super) fn put_budget(&self, cache_key: &str, budget: LiteLLMBudgetTable) {
        let mut budgets = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
        budgets.insert(
            cache_key.to_owned(),
            serde_json::to_value(budget).expect("the budget fixture serializes"),
        );
    }

    pub(super) fn put_end_user(&self, cache_key: &str, end_user: LiteLLMEndUserTable) {
        let encoded = serde_json::to_value(end_user).expect("the end-user fixture serializes");
        let seeded = self
            .end_users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key.to_owned(), encoded);
        assert!(seeded.is_none(), "an end-user fixture is seeded once");
    }

    pub(super) fn put_registry(&self, ids: &[&str]) {
        let answer = Json::List(ids.iter().map(|id| Json::Text((*id).to_owned())).collect());
        *self.registry.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
    }

    pub(super) fn stored_end_user(&self, cache_key: &str) -> Option<LiteLLMEndUserTable> {
        let stored = self
            .end_users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(cache_key)
            .cloned();
        stored.map(|row| serde_json::from_value(row).expect("the stored end-user row decodes"))
    }

    pub(super) fn reads(&self) -> Vec<String> {
        entries(&self.reads)
    }

    pub(super) fn writes(&self) -> Vec<String> {
        entries(&self.writes)
    }
}

#[async_trait]
impl RegistryCache for ScriptedCache {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        push(&self.reads, format!("registry:{cache_key}"));
        if self.fail_reads == Some(Surface::Registry) {
            return Err(cache_error());
        }
        let registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(registry.clone())
    }

    async fn write(&self, cache_key: &str, _answer: &Json, ttl: f64) -> Result<(), AuthCacheError> {
        push(&self.writes, format!("registry:{cache_key}:{ttl}"));
        if self.fail_writes == Some(Surface::Registry) {
            return Err(cache_error());
        }
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        self.ttl
    }
}

#[async_trait]
impl EndUserCache for ScriptedCache {
    async fn read_row<Row>(&self, cache_key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned + Send + 'static,
    {
        let budget = cache_key.starts_with(DEFAULT_END_USER_BUDGET_PREFIX)
            || cache_key.starts_with(TEAM_MEMBER_BUDGET_PREFIX);
        let (surface, stored) = if budget {
            push(&self.reads, format!("budget:{cache_key}"));
            let stored = self
                .budgets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(cache_key)
                .cloned();
            (Surface::Budget, stored)
        } else {
            push(&self.reads, format!("end_user:{cache_key}"));
            let stored = self
                .end_users
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(cache_key)
                .cloned();
            (Surface::EndUser, stored)
        };
        if self.fail_reads == Some(surface) {
            return Err(payload_error());
        }
        stored
            .map(serde_json::from_value)
            .transpose()
            .map_err(PayloadError::Json)
    }

    async fn write_row<Row>(
        &self,
        cache_key: &str,
        row: &Row,
        ttl: f64,
    ) -> Result<(), AuthCacheError>
    where
        Row: Serialize + Sync,
    {
        let encoded = serde_json::to_value(row)
            .map_err(|source| AuthCacheError::from(PayloadError::Json(source)))?;
        let budget = cache_key.starts_with(DEFAULT_END_USER_BUDGET_PREFIX)
            || cache_key.starts_with(TEAM_MEMBER_BUDGET_PREFIX);
        let surface = if budget {
            Surface::Budget
        } else {
            Surface::EndUser
        };
        let table = if budget { "budget" } else { "end_user" };
        push(&self.writes, format!("{table}:{cache_key}:{ttl}"));
        if self.fail_writes == Some(surface) {
            return Err(cache_error());
        }
        let mut rows = if budget {
            self.budgets.lock().unwrap_or_else(PoisonError::into_inner)
        } else {
            self.end_users
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        rows.insert(cache_key.to_owned(), encoded);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) enum BudgetAnswer {
    Row(Option<LiteLLMBudgetTable>),
    QueryFailure,
    DecodeFailure,
}

pub(super) struct ScriptedBudgetRows {
    pub(super) answers: Mutex<BTreeMap<String, BudgetAnswer>>,
    pub(super) fallback: BudgetAnswer,
    pub(super) queries: Journal,
}

impl ScriptedBudgetRows {
    pub(super) fn answering(fallback: BudgetAnswer) -> Self {
        Self {
            answers: Mutex::new(BTreeMap::new()),
            fallback,
            queries: journal(),
        }
    }

    pub(super) fn set(&self, budget_id: &str, answer: BudgetAnswer) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(budget_id.to_owned(), answer);
    }

    pub(super) fn queries(&self) -> Vec<String> {
        entries(&self.queries)
    }
}

#[async_trait]
impl BudgetRows for ScriptedBudgetRows {
    async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetFetchError> {
        push(&self.queries, budget_id.to_owned());
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(budget_id)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone());
        match answer {
            BudgetAnswer::Row(row) => Ok(row),
            BudgetAnswer::QueryFailure => Err(BudgetFetchError::Query(Box::new(query_failure()))),
            BudgetAnswer::DecodeFailure => {
                Err(BudgetFetchError::Decode(Box::new(decode_failure())))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum EndUserAnswer {
    Row(Option<LiteLLMEndUserTable>),
    Failure,
}

pub(super) struct ScriptedEndUserRows {
    pub(super) answers: Mutex<BTreeMap<String, EndUserAnswer>>,
    pub(super) fallback: EndUserAnswer,
    pub(super) queries: Journal,
}

impl ScriptedEndUserRows {
    pub(super) fn answering(fallback: EndUserAnswer) -> Self {
        Self {
            answers: Mutex::new(BTreeMap::new()),
            fallback,
            queries: journal(),
        }
    }

    pub(super) fn set(&self, end_user_id: &str, answer: EndUserAnswer) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(end_user_id.to_owned(), answer);
    }

    pub(super) fn queries(&self) -> Vec<String> {
        entries(&self.queries)
    }
}

#[async_trait]
impl EndUserRows for ScriptedEndUserRows {
    async fn find_end_user(
        &self,
        end_user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFetchError> {
        push(&self.queries, end_user_id.to_owned());
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(end_user_id)
            .cloned();
        match answer {
            Some(EndUserAnswer::Row(row)) => Ok(row),
            Some(EndUserAnswer::Failure) => Err(EndUserFetchError::Read(Box::new(query_failure()))),
            None => match self.fallback.clone() {
                EndUserAnswer::Row(row) => Ok(row),
                EndUserAnswer::Failure => Err(EndUserFetchError::Read(Box::new(query_failure()))),
            },
        }
    }
}

pub(super) struct SpendCache {
    pub(super) memory: Mutex<BTreeMap<String, ReseedCacheValue>>,
}

impl SpendCache {
    pub(super) fn new() -> Self {
        Self {
            memory: Mutex::new(BTreeMap::new()),
        }
    }
}

impl ReseedCache for SpendCache {
    type Error = SpendFailure;

    fn redis_configured(&self) -> bool {
        false
    }

    fn redis_get(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, SpendFailure>> + Send {
        ready(Ok(None))
    }

    fn redis_set_nx(
        &self,
        _counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, SpendFailure>> + Send {
        ready(Ok(None))
    }

    fn memory_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, SpendFailure>> + Send {
        let value = self
            .memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(counter_key)
            .cloned();
        ready(Ok(value))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), SpendFailure>> + Send {
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.insert(counter_key.to_owned(), ReseedCacheValue::Number(value));
        drop(memory);
        ready(Ok(()))
    }

    fn increment_local(
        &self,
        _counter_key: &str,
        _increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, SpendFailure>> + Send {
        ready(Ok(None))
    }
}

impl SpendReadCache for SpendCache {
    fn memory_set_ttl(
        &self,
        counter_key: &str,
        value: Decimal,
        _ttl_seconds: Option<f64>,
    ) -> Result<(), SpendFailure> {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(counter_key.to_owned(), ReseedCacheValue::Number(value));
        Ok(())
    }

    fn redis_set_max(
        &self,
        _counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, SpendFailure>> + Send {
        ready(Ok(None))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted spend store failed")]
pub(super) struct SpendFailure;

pub(super) struct JournalBatch {
    pub(super) answer: Mutex<Option<CounterValue>>,
    pub(super) read_fails: bool,
    pub(super) reads: Journal,
}

impl JournalBatch {
    pub(super) fn answering(answer: Option<CounterValue>) -> Self {
        Self {
            answer: Mutex::new(answer),
            read_fails: false,
            reads: journal(),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            answer: Mutex::new(None),
            read_fails: true,
            reads: journal(),
        }
    }

    pub(super) fn reads(&self) -> Vec<String> {
        entries(&self.reads)
    }
}

impl ActiveSpendBatch for JournalBatch {
    fn read(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        push(&self.reads, counter_key.to_owned());
        if self.read_fails {
            return ready(Err(SpendCounterBatchError::Cancelled));
        }
        let answer = *self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(answer))
    }

    fn record(&self, _counter_key: &str, _value: Decimal) {}
}

impl SpendBatch for JournalBatch {
    fn forget(&self, _counter_key: &str) {}
}

pub(super) struct ScriptedSpendRows {
    pub(super) end_user: Option<Decimal>,
    pub(super) lookups: Journal,
}

impl ScriptedSpendRows {
    pub(super) fn new(end_user: Option<Decimal>) -> Self {
        Self {
            end_user,
            lookups: journal(),
        }
    }

    pub(super) fn lookups(&self) -> Vec<String> {
        entries(&self.lookups)
    }
}

impl DbSpendCounterRows for ScriptedSpendRows {
    type Error = SpendFailure;

    fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<SpendRow>, SpendFailure>> + Send {
        push(&self.lookups, format!("{lookup:?}"));
        let spend = match lookup {
            SpendRowLookup::EndUser(_) => self.end_user,
            SpendRowLookup::Key(_)
            | SpendRowLookup::TeamMember { .. }
            | SpendRowLookup::Team(_)
            | SpendRowLookup::User(_)
            | SpendRowLookup::Organization(_)
            | SpendRowLookup::Project(_) => None,
        };
        ready(Ok(spend.map(|amount| SpendRow {
            spend: Some(amount),
        })))
    }
}

impl SpendCounterRows for ScriptedSpendRows {
    type Error = SpendFailure;

    fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, SpendFailure>> + Send {
        ready(Ok(None))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, SpendFailure>> + Send {
        ready(Ok(Vec::new()))
    }
}

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

pub(super) fn reseed_over<Rows>(rows: Option<Rows>) -> SpendCounterReseed<Rows>
where
    Rows: SpendCounterRows,
{
    SpendCounterReseed {
        rows: rows.map(Arc::new),
        locks: LockRegistry::from(10_000_i64),
        clock: Box::new(UptimeClock {
            started: Instant::now(),
        }),
        durations: Arc::new(NoDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: 25,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        deadline_seconds: 5.0_f64,
    }
}

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

pub(super) type TestCache = AuthCache<DualCache<LocalMemoryOps>>;

// the production end-user cache over local memory, for the Postgres lookup
pub(super) fn test_cache() -> TestCache {
    gateway_cache(
        local_memory(),
        local_memory(),
        None,
        dual_settings(),
        45.0_f64,
    )
}

// a real dual cache without Redis, for the Postgres spend reads
pub(super) fn spend_dual_cache() -> DualCache<LocalMemoryOps> {
    DualCache::new(local_memory(), None, dual_settings())
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggedEvent {
                level: *event.metadata().level(),
                message: fields.message,
            });
    }
}

pub(super) fn capture_events() -> (EventLog, DefaultGuard) {
    let log = EventLog::default();
    let guard = set_default(registry().with(log.clone()));
    (log, guard)
}

pub(super) fn events_at(log: &EventLog, level: Level) -> Vec<String> {
    log.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|event| event.level == level)
        .map(|event| event.message.clone())
        .collect()
}
