use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool, Runtime, Timeouts};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

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
use crate::postgres_store::session::PostgresStore;

use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::rows::DueIdentity;
use crate::identity_budget_reset::service::IdentityBudgetReset;
use crate::identity_budget_reset::service_hooks::{PhaseSuccessEvent, PhaseSuccessSink};
use crate::identity_budget_reset::write_transactional::ResetWrite;

pub(crate) const IDENTITY_TABLES: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    expires TIMESTAMP(3),
    budget_duration TEXT,
    budget_reset_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    budget_duration TEXT,
    budget_reset_at TIMESTAMP(3)
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn configured_url() -> Option<SecretString> {
    Environment::parse_from(["identity-budget-reset-tests"])
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
    provision(vec![IDENTITY_TABLES.to_owned()]).await
}

pub(crate) async fn provision(statements: Vec<String>) -> (PostgresFixture, PostgresStore, Client) {
    let fixture = PostgresFixture::provision_with_statements(configured_url().as_ref(), statements)
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
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, second).unwrap()
}

pub(crate) fn naive(second: u32) -> NaiveDateTime {
    moment(second).naive_utc()
}

pub(crate) fn past() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2025-12-30 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
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

pub(crate) async fn reset_spend_count(client: &Client) -> i64 {
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM \"LiteLLM_VerificationToken\" WHERE spend = 0",
            &[],
        )
        .await
        .unwrap();
    row.get(0)
}

pub(crate) fn identifiers(due: &[DueIdentity]) -> Vec<String> {
    due.iter().map(|row| row.identifier.clone()).collect()
}

pub(crate) fn write(
    identifier: &str,
    reset_at: Option<DateTime<Utc>>,
    decrement: Decimal,
) -> ResetWrite {
    ResetWrite {
        identifier: identifier.to_owned(),
        budget_reset_at: reset_at,
        spend_decrement: decrement,
    }
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
}

impl TestSchedule {
    pub(crate) fn fixed(next: DateTime<Utc>) -> Self {
        Self {
            next,
            failure: None,
        }
    }

    pub(crate) fn failing_for(next: DateTime<Utc>, duration: &str, message: &str) -> Self {
        Self {
            next,
            failure: Some(ScheduleFailure {
                duration: duration.to_owned(),
                message: message.to_owned(),
            }),
        }
    }
}

impl ResetSchedule for TestSchedule {
    type Failure = ScheduleFault;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, ScheduleFault> {
        match &self.failure {
            Some(failure) if failure.duration == budget_duration => Err(ScheduleFault {
                message: failure.message.clone(),
            }),
            _ => Ok(self.next),
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

pub(crate) type ServiceUnderTest<'service, Cache, Reconnect> = IdentityBudgetReset<
    'service,
    TestSchedule,
    Cache,
    RecordingFailures,
    RecordingSuccesses,
    Reconnect,
    TestClock,
>;

pub(crate) struct Harness<Cache, Reconnect> {
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

impl<Cache, Reconnect> Harness<Cache, Reconnect> {
    pub(crate) fn new(
        store: PostgresStore,
        schedule: TestSchedule,
        cache: Cache,
        reconnect: Reconnect,
    ) -> Self {
        Self {
            store,
            limits: ResetLimits::default(),
            rollover: RolloverPolicy::default(),
            schedule,
            cache,
            failures: RecordingFailures::default(),
            successes: RecordingSuccesses::default(),
            clock: TestClock::fixed(moment(0)),
            reconnect: Some(reconnect),
        }
    }

    pub(crate) fn service(&mut self) -> ServiceUnderTest<'_, Cache, Reconnect> {
        IdentityBudgetReset {
            store: self.store.clone(),
            limits: self.limits,
            rollover: self.rollover,
            schedule: &self.schedule,
            cache: &self.cache,
            failure: &self.failures,
            success: &self.successes,
            reconnect: self.reconnect.as_mut(),
            clock: &self.clock,
        }
    }

    pub(crate) async fn run(&mut self, phase: IdentityPhase)
    where
        Cache: ResetCache,
        Reconnect: DbReconnect + Send,
    {
        self.service().reset(phase).await;
    }
}
