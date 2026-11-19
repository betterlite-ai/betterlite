use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
use crate::budget_tier_sweep::service::BudgetTierSweep;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::service_hooks::{PhaseSuccessEvent, PhaseSuccessSink};
use crate::postgres_store::session::PostgresStore;

const TABLES: &str = "CREATE TABLE \"LiteLLM_BudgetTable\" (budget_id TEXT PRIMARY KEY, max_budget DOUBLE PRECISION, soft_budget DOUBLE PRECISION, max_parallel_requests INTEGER, tpm_limit BIGINT, rpm_limit BIGINT, model_max_budget JSONB, budget_duration TEXT, budget_reset_at TIMESTAMP(3), created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP); CREATE TABLE \"LiteLLM_TeamMembership\" (user_id TEXT, team_id TEXT, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0, PRIMARY KEY (user_id, team_id)); CREATE TABLE \"LiteLLM_VerificationToken\" (token TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0, budget_duration TEXT); CREATE TABLE \"LiteLLM_OrganizationTable\" (organization_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_TagTable\" (tag_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ModelAccessGroupBudgetTable\" (access_group_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ProjectTable\" (project_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_EndUserTable\" (user_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0);";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn configured_url() -> Option<SecretString> {
    Environment::parse_from(["budget-tier-sweep-tests"])
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
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, second).unwrap()
}

pub(crate) fn naive(second: u32) -> NaiveDateTime {
    moment(second).naive_utc()
}

pub(crate) fn past() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2025-12-30 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
}

pub(crate) async fn insert_tier(
    client: &Client,
    budget_id: &str,
    duration: Option<&str>,
    reset_at: Option<NaiveDateTime>,
) {
    client
        .execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, budget_duration, budget_reset_at) VALUES ($1, $2, $3)",
            &[&budget_id, &duration, &reset_at],
        )
        .await
        .unwrap();
}

pub(crate) async fn spend_of(client: &Client, table: &str, column: &str, id: &str) -> Decimal {
    let sql = format!("SELECT spend::numeric AS spend FROM \"{table}\" WHERE {column} = $1");
    let row = client.query_one(sql.as_str(), &[&id]).await.unwrap();
    row.get("spend")
}

pub(crate) async fn reset_at_of(client: &Client, budget_id: &str) -> Option<NaiveDateTime> {
    client
        .query_one(
            "SELECT budget_reset_at FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
            &[&budget_id],
        )
        .await
        .unwrap()
        .get(0)
}

pub(crate) async fn unreset_count(client: &Client) -> i64 {
    client
        .query_one(
            "SELECT COUNT(*) FROM \"LiteLLM_BudgetTable\" WHERE budget_reset_at IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

pub(crate) async fn reject_budget_updates(client: &Client) {
    client
        .batch_execute(
            "CREATE FUNCTION reject_budget_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'budget table is locked'; END $$; CREATE TRIGGER reject_budget_update BEFORE UPDATE ON \"LiteLLM_BudgetTable\" FOR EACH ROW EXECUTE FUNCTION reject_budget_update();",
        )
        .await
        .unwrap();
}

pub(crate) async fn retire_enduser_table(client: &Client) {
    client
        .batch_execute(
            "ALTER TABLE \"LiteLLM_EndUserTable\" RENAME TO \"LiteLLM_EndUserTable_retired\"",
        )
        .await
        .unwrap();
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
    durations: Mutex<Vec<String>>,
}

impl TestSchedule {
    pub(crate) fn fixed(next: DateTime<Utc>) -> Self {
        Self {
            next,
            failure: None,
            durations: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(next: DateTime<Utc>, duration: &str, message: &str) -> Self {
        Self {
            next,
            failure: Some(ScheduleFailure {
                duration: duration.to_owned(),
                message: message.to_owned(),
            }),
            durations: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn durations(&self) -> Vec<String> {
        self.durations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResetSchedule for TestSchedule {
    type Failure = ScheduleFault;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, ScheduleFault> {
        self.durations
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

pub(crate) struct TestClock {
    scripted: Mutex<VecDeque<DateTime<Utc>>>,
    fallback: DateTime<Utc>,
}

impl TestClock {
    pub(crate) fn fixed(fallback: DateTime<Utc>) -> Self {
        Self::scripted(Vec::new(), fallback)
    }

    pub(crate) fn scripted(scripted: Vec<DateTime<Utc>>, fallback: DateTime<Utc>) -> Self {
        Self {
            scripted: Mutex::new(VecDeque::from(scripted)),
            fallback,
        }
    }
}

impl RefreshClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
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

struct CacheState {
    calls: Mutex<Vec<CacheCall>>,
    seen: Mutex<Vec<Decimal>>,
    probe: Option<PostgresStore>,
    sabotage: Option<PostgresStore>,
    sabotaged: AtomicBool,
}

impl CacheState {
    fn new(probe: Option<PostgresStore>, sabotage: Option<PostgresStore>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            probe,
            sabotage,
            sabotaged: AtomicBool::new(false),
        }
    }

    // Every spend-counter drop reads the committed spend and the first retires the table, so an early invalidation shows nonzero spend and the walk's next page fails.
    async fn after_counter_drop(&self) {
        if let Some(store) = self.probe.as_ref() {
            let client = store.session().await.unwrap();
            let row = client
                .query_one(
                    "SELECT spend::numeric AS spend FROM \"LiteLLM_EndUserTable\" WHERE user_id = 'customer-a'",
                    &[],
                )
                .await
                .unwrap();
            let spend: Decimal = row.get("spend");
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(spend);
        }
        let Some(store) = self.sabotage.as_ref() else {
            return;
        };
        if self.sabotaged.swap(true, Ordering::SeqCst) {
            return;
        }
        let client = store.session().await.unwrap();
        retire_enduser_table(&client).await;
    }
}

#[derive(Clone)]
pub(crate) struct RecordingCache {
    state: Arc<CacheState>,
}

impl Default for RecordingCache {
    fn default() -> Self {
        Self {
            state: Arc::new(CacheState::new(None, None)),
        }
    }
}

impl RecordingCache {
    pub(crate) fn probing(store: PostgresStore) -> Self {
        Self {
            state: Arc::new(CacheState::new(Some(store), None)),
        }
    }

    pub(crate) fn sabotaging(store: PostgresStore) -> Self {
        Self {
            state: Arc::new(CacheState::new(None, Some(store))),
        }
    }

    pub(crate) fn calls(&self) -> Vec<CacheCall> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn seen(&self) -> Vec<Decimal> {
        self.state
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, sink: InvalidatedCache, keys: &[String]) {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheCall {
                sink,
                keys: keys.to_vec(),
            });
    }

    fn delete<'cache>(
        &'cache self,
        sink: InvalidatedCache,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        Box::pin(async move {
            self.record(sink, keys);
            if sink == InvalidatedCache::SpendCounter {
                self.state.after_counter_drop().await;
            }
            Ok(())
        })
    }
}

impl ResetCache for RecordingCache {
    type Failure = CacheFault;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        self.delete(InvalidatedCache::SpendCounter, keys)
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        self.delete(InvalidatedCache::UserApiKeyCache, keys)
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

pub(crate) type ServiceUnderTest<'service, Cache, Reconnect> = BudgetTierSweep<
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
    pub(crate) default_enduser_budget_id: Option<String>,
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
            default_enduser_budget_id: None,
            schedule,
            cache,
            failures: RecordingFailures::default(),
            successes: RecordingSuccesses::default(),
            clock: TestClock::fixed(moment(0)),
            reconnect: Some(reconnect),
        }
    }

    pub(crate) fn service(&mut self) -> ServiceUnderTest<'_, Cache, Reconnect> {
        BudgetTierSweep {
            store: self.store.clone(),
            limits: self.limits,
            rollover: self.rollover,
            default_enduser_budget_id: self.default_enduser_budget_id.as_deref(),
            schedule: &self.schedule,
            cache: &self.cache,
            failure: &self.failures,
            success: &self.successes,
            reconnect: self.reconnect.as_mut(),
            clock: &self.clock,
        }
    }

    pub(crate) async fn run(&mut self)
    where
        Cache: ResetCache,
        Reconnect: DbReconnect + Send,
    {
        self.service().reset().await;
    }
}
