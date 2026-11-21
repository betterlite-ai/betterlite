use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::slice::from_ref;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool, Runtime, Timeouts};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::budget_permission_rows::JsonData;
use crate::budget_reset_policy::cache_invalidation::{
    CacheDeleteFuture, InvalidatedCache, ResetCache,
};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::phase_failure::{PhaseFailureEvent, PhaseFailureSink};
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::service_hooks::{PhaseSuccessEvent, PhaseSuccessSink};
use crate::postgres_store::session::PostgresStore;
use crate::search_tools::convert::isoformat;
use crate::team_window_reset::cache::{CounterReadFuture, CounterWriteFuture, WindowCounterCache};
use crate::team_window_reset::pagination::BudgetWindowReset;
use crate::team_window_reset::team_resets::TeamBudgetReset;
use crate::team_window_reset::window_cache_roll::{WindowDurationSeconds, WindowResetter};
use crate::window_spend::store::WindowSpendStore;

pub(crate) const TABLES: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    budget_limits JSONB,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    budget_duration TEXT,
    budget_reset_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    budget_limits JSONB,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    budget_duration TEXT,
    budget_reset_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_BudgetWindowSpend" (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    window_duration TEXT NOT NULL,
    window_start TIMESTAMP(3) NOT NULL,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL,
    PRIMARY KEY (entity_type, entity_id, window_duration)
);
"#;

pub(crate) const KEY_TABLE: &str = "LiteLLM_VerificationToken";
pub(crate) const KEY_COLUMN: &str = "token";
pub(crate) const TEAM_TABLE: &str = "LiteLLM_TeamTable";
pub(crate) const TEAM_COLUMN: &str = "team_id";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn configured_url() -> Option<SecretString> {
    Environment::parse_from(["team-window-reset-tests"])
        .url
        .map(SecretString::from)
}

pub(crate) fn pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager).max_size(4).build().unwrap()
}

// One connection and a short wait: a test that holds it makes the next session wait and then give up with a pool timeout.
pub(crate) fn serial_pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager)
        .max_size(1)
        .timeouts(Timeouts::wait_millis(100))
        .runtime(Runtime::Tokio1)
        .build()
        .unwrap()
}

pub(crate) async fn schema() -> (PostgresFixture, PostgresStore, Client) {
    provision(vec![TABLES.to_owned()]).await
}

pub(crate) async fn provision(statements: Vec<String>) -> (PostgresFixture, PostgresStore, Client) {
    let url = configured_url();
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool(&fixture.options.url)),
    };
    let client = store.session().await.unwrap();
    (fixture, store, client)
}

pub(crate) fn close_store(store: PostgresStore) {
    if let Some(pool) = store.pool {
        pool.close();
    }
}

pub(crate) async fn finish(fixture: PostgresFixture, store: PostgresStore, client: Client) {
    drop(client);
    close_store(store);
    fixture.close_async().await.unwrap();
}

pub(crate) fn moment(second: u32) -> DateTime<Utc> {
    let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let offset = TimeDelta::try_seconds(i64::from(second)).unwrap();
    base.checked_add_signed(offset).unwrap()
}

pub(crate) fn naive(second: u32) -> NaiveDateTime {
    moment(second).naive_utc()
}

pub(crate) fn past() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2025-12-30 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
}

pub(crate) fn next_text() -> String {
    isoformat(moment(3600))
}

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal amount")
}

#[expect(
    clippy::too_many_arguments,
    reason = "one argument per column of the team row a test seeds"
)]
pub(crate) async fn insert_team(
    client: &Client,
    team_id: &str,
    spend: f64,
    max_budget: Option<f64>,
    budget_duration: Option<&str>,
    budget_reset_at: Option<NaiveDateTime>,
) {
    let sql = "INSERT INTO \"LiteLLM_TeamTable\" (team_id, spend, max_budget, budget_duration, budget_reset_at) VALUES ($1, $2, $3, $4, $5)";
    client
        .execute(
            sql,
            &[
                &team_id,
                &spend,
                &max_budget,
                &budget_duration,
                &budget_reset_at,
            ],
        )
        .await
        .expect("the team row");
}

pub(crate) async fn insert_windows(
    client: &Client,
    table: &str,
    id_column: &str,
    id: &str,
    windows: &[JsonData],
) {
    let sql = format!(
        "INSERT INTO \"{table}\" ({id_column}, budget_limits) VALUES ($1, $2::text::jsonb)"
    );
    let payload = serde_json::to_string(windows).expect("the windows json");
    client
        .execute(sql.as_str(), &[&id, &payload])
        .await
        .expect("the windowed row");
}

// A reset_at earlier than every test clock, so a window carrying it is expired.
pub(crate) const EXPIRED_TEXT: &str = "2025-12-30T00:00:00+00:00";

pub(crate) async fn insert_expired_windows(
    client: &Client,
    table: &str,
    id_column: &str,
    identifiers: &[&str],
) {
    let window = window_json("1h", None, Some(EXPIRED_TEXT));
    for identifier in identifiers {
        insert_windows(client, table, id_column, identifier, from_ref(&window)).await;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one argument per column of the window spend row a test seeds"
)]
pub(crate) async fn seed_window_spend(
    client: &Client,
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
    window_start: NaiveDateTime,
    spend: f64,
) {
    let sql = "INSERT INTO \"LiteLLM_BudgetWindowSpend\" (entity_type, entity_id, window_duration, window_start, spend, updated_at) VALUES ($1, $2, $3, $4, $5, $4)";
    client
        .execute(
            sql,
            &[
                &entity_type,
                &entity_id,
                &window_duration,
                &window_start,
                &spend,
            ],
        )
        .await
        .expect("the window spend row");
}

pub(crate) async fn spend_of(client: &Client, table: &str, column: &str, id: &str) -> f64 {
    let sql = format!("SELECT spend FROM \"{table}\" WHERE {column} = $1");
    let row = client.query_one(sql.as_str(), &[&id]).await.unwrap();
    row.get(0)
}

pub(crate) async fn assert_spend(client: &Client, table: &str, column: &str, id: &str, spent: f64) {
    let actual = spend_of(client, table, column, id).await;
    assert_eq!(actual.to_bits(), spent.to_bits());
}

pub(crate) async fn reset_at_of(
    client: &Client,
    table: &str,
    column: &str,
    id: &str,
) -> Option<NaiveDateTime> {
    let sql = format!("SELECT budget_reset_at FROM \"{table}\" WHERE {column} = $1");
    let row = client.query_one(sql.as_str(), &[&id]).await.unwrap();
    row.get(0)
}

pub(crate) async fn windows_text(
    client: &Client,
    table: &str,
    id_column: &str,
    id: &str,
) -> Vec<JsonData> {
    let sql = format!("SELECT budget_limits::text FROM \"{table}\" WHERE {id_column} = $1");
    let row = client.query_one(sql.as_str(), &[&id]).await.unwrap();
    let text: String = row.get(0);
    decode_windows(&text)
}

pub(crate) async fn budget_limits_type(
    client: &Client,
    table: &str,
    id_column: &str,
    id: &str,
) -> String {
    let statement =
        format!("SELECT jsonb_typeof(budget_limits) FROM \"{table}\" WHERE {id_column} = $1");
    match client.query_one(statement.as_str(), &[&id]).await {
        Ok(row) => row.get(0),
        Err(error) => panic!("the windowed row: {error}"),
    }
}

pub(crate) async fn window_spend_row(
    client: &Client,
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
) -> Option<(NaiveDateTime, f64, NaiveDateTime)> {
    let sql = "SELECT window_start, spend, updated_at FROM \"LiteLLM_BudgetWindowSpend\" WHERE entity_type = $1 AND entity_id = $2 AND window_duration = $3";
    client
        .query_opt(sql, &[&entity_type, &entity_id, &window_duration])
        .await
        .unwrap()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
}

pub(crate) fn decode_windows(text: &str) -> Vec<JsonData> {
    let value: JsonData = serde_json::from_str(text).expect("the stored budget_limits");
    match value {
        JsonData::Array(windows) => windows,
        JsonData::String(inner) => {
            let listed: JsonData = serde_json::from_str(&inner).expect("the stored list");
            match listed {
                JsonData::Array(windows) => windows,
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::String(_)
                | JsonData::Object(_) => panic!("the stored budget_limits is not a list"),
            }
        }
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Object(_) => {
            panic!("the stored budget_limits is not a list")
        }
    }
}

pub(crate) fn window_json(
    duration: &str,
    max_budget: Option<&str>,
    reset_at: Option<&str>,
) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert(
        "budget_duration".to_owned(),
        JsonData::String(duration.to_owned()),
    );
    if let Some(limit) = max_budget {
        let number = serde_json::Number::from_str(limit).expect("a json number");
        fields.insert("max_budget".to_owned(), JsonData::Number(number));
    }
    if let Some(text) = reset_at {
        fields.insert("reset_at".to_owned(), JsonData::String(text.to_owned()));
    }
    JsonData::Object(fields)
}

pub(crate) fn window_field<'json>(window: &'json JsonData, name: &str) -> Option<&'json JsonData> {
    match window {
        JsonData::Object(fields) => fields.get(name),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => None,
    }
}

pub(crate) fn window_text(window: &JsonData, name: &str) -> Option<String> {
    let JsonData::String(text) = window_field(window, name)? else {
        return None;
    };
    Some(text.clone())
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ScheduleFault {
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleFailure {
    pub(crate) duration: String,
    pub(crate) message: String,
}

pub(crate) struct TestSchedule {
    pub(crate) next: DateTime<Utc>,
    pub(crate) failure: Option<ScheduleFailure>,
    requested: Mutex<Vec<String>>,
}

impl TestSchedule {
    pub(crate) fn fixed(next: DateTime<Utc>) -> Self {
        Self {
            next,
            failure: None,
            requested: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing_for(next: DateTime<Utc>, duration: &str, message: &str) -> Self {
        Self {
            next,
            failure: Some(ScheduleFailure {
                duration: duration.to_owned(),
                message: message.to_owned(),
            }),
            requested: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requested(&self) -> Vec<String> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResetSchedule for TestSchedule {
    type Failure = ScheduleFault;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, ScheduleFault> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(budget_duration.to_owned());
        match &self.failure {
            Some(failure) if failure.duration == budget_duration => Err(ScheduleFault {
                message: failure.message.clone(),
            }),
            _ => Ok(self.next),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct DurationFault {
    pub(crate) message: String,
}

pub(crate) struct TestDurations {
    pub(crate) seconds: i64,
    pub(crate) failure: Option<String>,
    requested: Mutex<Vec<String>>,
}

impl TestDurations {
    pub(crate) fn fixed(seconds: i64) -> Self {
        Self {
            seconds,
            failure: None,
            requested: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing_for(seconds: i64, duration: &str) -> Self {
        Self {
            seconds,
            failure: Some(duration.to_owned()),
            requested: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requested(&self) -> Vec<String> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WindowDurationSeconds for TestDurations {
    type Failure = DurationFault;

    fn duration_seconds(&self, budget_duration: &str) -> Result<i64, DurationFault> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(budget_duration.to_owned());
        match &self.failure {
            Some(duration) if duration == budget_duration => Err(DurationFault {
                message: format!("cannot parse {budget_duration}"),
            }),
            _ => Ok(self.seconds),
        }
    }
}

pub(crate) struct TestClock {
    scripted: Mutex<VecDeque<DateTime<Utc>>>,
    fallback: DateTime<Utc>,
    calls: AtomicUsize,
}

impl TestClock {
    pub(crate) fn fixed(fallback: DateTime<Utc>) -> Self {
        Self::scripted(Vec::new(), fallback)
    }

    pub(crate) fn scripted(scripted: Vec<DateTime<Utc>>, fallback: DateTime<Utc>) -> Self {
        Self {
            scripted: Mutex::new(VecDeque::from(scripted)),
            fallback,
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RefreshClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.scripted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingFailures(Arc<Mutex<Vec<PhaseFailureEvent>>>);

impl RecordingFailures {
    pub(crate) fn events(&self) -> Vec<PhaseFailureEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PhaseFailureSink for RecordingFailures {
    fn emit(&self, event: PhaseFailureEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingSuccesses(Arc<Mutex<Vec<PhaseSuccessEvent>>>);

impl RecordingSuccesses {
    pub(crate) fn events(&self) -> Vec<PhaseSuccessEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PhaseSuccessSink for RecordingSuccesses {
    fn emit(&self, event: PhaseSuccessEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the cache refused the entries")]
pub(crate) struct CacheFault;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheCall {
    pub(crate) sink: InvalidatedCache,
    pub(crate) keys: Vec<String>,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingCache(Arc<Mutex<Vec<CacheCall>>>);

impl RecordingCache {
    pub(crate) fn calls(&self) -> Vec<CacheCall> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, sink: InvalidatedCache, keys: &[String]) -> CacheDeleteFuture<'_, CacheFault> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheCall {
                sink,
                keys: keys.to_vec(),
            });
        Box::pin(async move { Ok(()) })
    }
}

impl ResetCache for RecordingCache {
    type Failure = CacheFault;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        self.record(InvalidatedCache::SpendCounter, keys)
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        self.record(InvalidatedCache::UserApiKeyCache, keys)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the counter cache refused")]
pub(crate) struct CounterFault;

#[derive(Default)]
pub(crate) struct FakeCounterCache {
    counters: Mutex<BTreeMap<String, Option<Decimal>>>,
    memory: Mutex<BTreeMap<String, Decimal>>,
    redis: Mutex<BTreeMap<String, Decimal>>,
    read_keys: Mutex<Vec<String>>,
    redis_failures: AtomicBool,
    read_failures: AtomicBool,
}

impl FakeCounterCache {
    pub(crate) fn with_counter(key: &str, value: Option<Decimal>) -> Self {
        let cache = Self::default();
        cache
            .counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        cache
    }

    pub(crate) fn failing_redis() -> Self {
        let cache = Self::default();
        cache.redis_failures.store(true, Ordering::SeqCst);
        cache
    }

    pub(crate) fn failing_reads() -> Self {
        let cache = Self::default();
        cache.read_failures.store(true, Ordering::SeqCst);
        cache
    }

    pub(crate) fn memory_of(&self, key: &str) -> Option<Decimal> {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    pub(crate) fn redis_of(&self, key: &str) -> Option<Decimal> {
        self.redis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    pub(crate) fn read_keys(&self) -> Vec<String> {
        self.read_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WindowCounterCache for FakeCounterCache {
    type Failure = CounterFault;

    fn set_memory(&self, key: &str, value: Decimal) {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    fn set_redis<'cache>(
        &'cache self,
        key: &'cache str,
        value: Decimal,
    ) -> CounterWriteFuture<'cache, CounterFault> {
        Box::pin(async move {
            if self.redis_failures.load(Ordering::SeqCst) {
                return Err(CounterFault);
            }
            self.redis
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned(), value);
            Ok(())
        })
    }

    fn read<'cache>(&'cache self, key: &'cache str) -> CounterReadFuture<'cache, CounterFault> {
        let counters = &self.counters;
        let read_keys = &self.read_keys;
        let failing = self.read_failures.load(Ordering::SeqCst);
        Box::pin(async move {
            if failing {
                return Err(CounterFault);
            }
            read_keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(key.to_owned());
            Ok(counters
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .copied()
                .flatten())
        })
    }
}

pub(crate) struct CountingReconnect {
    pub(crate) succeeds: bool,
    pub(crate) held: Option<Client>,
    pub(crate) attempts: AtomicUsize,
    pub(crate) reasons: Mutex<Vec<String>>,
}

impl CountingReconnect {
    pub(crate) fn new(succeeds: bool) -> Self {
        Self {
            succeeds,
            held: None,
            attempts: AtomicUsize::new(0),
            reasons: Mutex::new(Vec::new()),
        }
    }

    // Releasing the connection the test holds is what a real reconnect would do; the next attempt then finds the pool healthy.
    pub(crate) fn releasing(held: Client) -> Self {
        let mut reconnect = Self::new(true);
        reconnect.held = Some(held);
        reconnect
    }

    pub(crate) fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    pub(crate) fn reasons(&self) -> Vec<String> {
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DbReconnect for CountingReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reason.to_owned());
        self.held = None;
        let succeeds = self.succeeds;
        Box::pin(async move { Ok(succeeds) })
    }
}

pub(crate) type TeamServiceUnderTest<'service, Cache, Reconnect> = TeamBudgetReset<
    'service,
    TestSchedule,
    Cache,
    RecordingFailures,
    RecordingSuccesses,
    Reconnect,
    TestClock,
>;

pub(crate) struct TeamHarness<Cache, Reconnect> {
    pub(crate) store: PostgresStore,
    pub(crate) limits: ResetLimits,
    pub(crate) rollover: RolloverPolicy,
    pub(crate) schedule: TestSchedule,
    pub(crate) cache: Cache,
    pub(crate) failures: RecordingFailures,
    pub(crate) successes: RecordingSuccesses,
    pub(crate) clock: TestClock,
    pub(crate) reconnect: Option<Reconnect>,
}

impl<Cache, Reconnect> TeamHarness<Cache, Reconnect> {
    pub(crate) fn new(
        store: PostgresStore,
        schedule: TestSchedule,
        cache: Cache,
        reconnect: Reconnect,
    ) -> Self {
        Self {
            failures: RecordingFailures::default(),
            successes: RecordingSuccesses::default(),
            clock: TestClock::fixed(moment(0)),
            reconnect: Some(reconnect),
            cache,
            schedule,
            rollover: RolloverPolicy::default(),
            limits: ResetLimits::default(),
            store,
        }
    }

    pub(crate) fn service(&mut self) -> TeamServiceUnderTest<'_, Cache, Reconnect> {
        TeamBudgetReset {
            clock: &self.clock,
            reconnect: self.reconnect.as_mut(),
            success: &self.successes,
            failure: &self.failures,
            cache: &self.cache,
            schedule: &self.schedule,
            rollover: self.rollover,
            limits: self.limits,
            store: self.store.clone(),
        }
    }

    pub(crate) async fn run(&mut self)
    where
        Cache: ResetCache,
        Reconnect: DbReconnect + Send,
    {
        self.service().reset_teams().await;
    }
}

pub(crate) struct WindowHarness<Cache, Durations, Reconnect> {
    pub(crate) store: PostgresStore,
    pub(crate) window_spend: WindowSpendStore,
    pub(crate) limits: ResetLimits,
    pub(crate) rollover: RolloverPolicy,
    pub(crate) schedule: TestSchedule,
    pub(crate) cache: Cache,
    pub(crate) durations: Durations,
    pub(crate) clock: TestClock,
    pub(crate) reconnect: Option<Reconnect>,
}

impl<Cache, Durations, Reconnect> WindowHarness<Cache, Durations, Reconnect> {
    pub(crate) fn new(
        store: PostgresStore,
        schedule: TestSchedule,
        cache: Cache,
        durations: Durations,
        reconnect: Reconnect,
    ) -> Self {
        let window_spend = WindowSpendStore::from_store(store.clone());
        let pending_reconnect = Some(reconnect);
        let clock = TestClock::fixed(moment(0));
        Self {
            store,
            window_spend,
            limits: ResetLimits::default(),
            rollover: RolloverPolicy::default(),
            schedule,
            cache,
            durations,
            clock,
            reconnect: pending_reconnect,
        }
    }

    pub(crate) fn resetter(&self) -> WindowResetter<'_, TestSchedule, Cache, Durations> {
        let now = self.clock.now();
        let store = &self.window_spend;
        let schedule = &self.schedule;
        let cache = &self.cache;
        let durations = &self.durations;
        WindowResetter {
            now,
            store,
            schedule,
            cache,
            durations,
            rollover: self.rollover,
        }
    }

    pub(crate) async fn run(&mut self)
    where
        Cache: WindowCounterCache,
        Durations: WindowDurationSeconds + Send + Sync,
        Reconnect: DbReconnect + Send,
    {
        let window_spend = WindowSpendStore::from_store(self.store.clone());
        BudgetWindowReset {
            clock: &self.clock,
            reconnect: self.reconnect.as_mut(),
            durations: &self.durations,
            cache: &self.cache,
            schedule: &self.schedule,
            rollover: self.rollover,
            limits: self.limits,
            window_spend,
            store: self.store.clone(),
        }
        .reset_windows()
        .await;
    }
}
