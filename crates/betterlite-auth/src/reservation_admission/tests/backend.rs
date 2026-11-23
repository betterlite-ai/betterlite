use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::spend_rows::rows::{
    SpendCounterRows as DbSpendCounterRows, SpendRow as DbSpendRow, SpendRowLookup,
};
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::reservation_admission::backend::SeedBatch;
use crate::reservation_admission::counters::{AdmissionCounters as _, WindowSeed};
use crate::reservation_settlement::backend::SpendCounterBackend;
use crate::reservation_settlement::counters::SettlementCounters as _;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::ActiveSpendBatch as _;
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::source::SourceSpendCache;

use super::support::{KEY, WINDOW, amount, window};

#[derive(Debug, thiserror::Error)]
#[error("the fake spend row read failed")]
struct RowsError;

// a spend table with no counter row, whose key windows sum to an empty log slice
struct FakeRows;

impl DbSpendCounterRows for FakeRows {
    type Error = RowsError;

    fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<DbSpendRow>, RowsError>> + Send {
        ready(Ok(None))
    }
}

impl SpendCounterRows for FakeRows {
    type Error = RowsError;

    fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, RowsError>> + Send {
        ready(Ok(None))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, RowsError>> + Send {
        ready(Ok(Vec::new()))
    }
}

struct FixedClock;

impl BudgetClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .expect("a fixture instant")
    }

    fn uptime(&self) -> Duration {
        Duration::ZERO
    }
}

struct FakeDurations;

impl DurationParser for FakeDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

// the auth cache's payload for every source key, which only the spend field is read from
struct SourceSpend(Option<Json>);

#[async_trait]
impl SourceSpendCache for SourceSpend {
    async fn cached(&self, _cache_key: &str) -> Option<Json> {
        self.0.clone()
    }
}

// a request batch that keeps every recorded value, so a read answers what a record wrote
#[derive(Default)]
struct RecordingBatch(Mutex<BTreeMap<String, Decimal>>);

#[async_trait]
impl CounterBatch for RecordingBatch {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        Ok(lock(&self.0)
            .get(counter_key)
            .copied()
            .map(CounterValue::Amount))
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        let _previous = lock(&self.0).insert(counter_key.to_owned(), value);
    }

    fn forget(&self, counter_key: &str) {
        let _removed = lock(&self.0).remove(counter_key);
    }
}

fn spend_payload(text: &str) -> Json {
    let mut fields = IndexMap::new();
    let _spend = fields.insert(
        "spend".to_owned(),
        Json::Number(text.parse().expect("a JSON number")),
    );
    Json::Object(fields)
}

fn local_memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    Arc::new(LocalMemoryOps::new(1_024, clock).expect("the local memory tier"))
}

fn settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    }
}

fn backend_fixture(
    source: Option<Json>,
) -> (
    SpendCounterWrites<DualCache<LocalMemoryOps>, FakeRows>,
    Arc<SpendCounterScope>,
) {
    let scope = Arc::new(SpendCounterScope::default());
    let batch_handle = Arc::clone(&scope);
    let batch: Arc<dyn CounterBatch> = batch_handle;
    let sources: Arc<dyn SourceSpendCache> = Arc::new(SourceSpend(source));
    let reseed = Arc::new(SpendCounterReseed {
        rows: Some(Arc::new(FakeRows)),
        locks: LockRegistry::from(10_000),
        clock: Box::new(FixedClock),
        durations: Arc::new(FakeDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: 25,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        deadline_seconds: 5.0,
    });
    let writes = SpendCounterWrites {
        layers: Arc::new(DualCache::new(local_memory(), None, settings())),
        batch,
        reseed,
        sources,
    };
    (writes, scope)
}

// seed_source: with no database floor the auth cache's spend becomes the counter's value
#[tokio::test]
async fn the_backend_seeds_a_source_counter_from_the_auth_cache() {
    let (writes, scope) = backend_fixture(Some(spend_payload("1.5")));
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    backend
        .seed_source(KEY, "k1")
        .await
        .expect("the source seed");
    let found = backend.read_cache_value(KEY).await.expect("the read");
    assert_eq!(found, Some(amount("1.5")));
}

// the window seed initialize.rs resolves from a counter whose entity and start are both set
fn window_seed(counter: &BudgetCounter) -> WindowSeed<'_> {
    WindowSeed {
        counter_key: &counter.counter_key,
        entity_type: &counter.entity_type,
        entity_id: counter
            .spend_log_entity_id
            .as_deref()
            .expect("the fixture's spend log entity"),
        window_duration: counter.window_duration.as_deref(),
        window_start: counter.window_start.expect("the fixture's window start"),
    }
}

// seed_window: a key window loads from its rows, while an entity without window rows reports cold
#[tokio::test]
async fn the_backend_seeds_a_window_counter_or_reports_a_cold_window() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    let key_window = window(WINDOW, "5");
    assert!(
        backend
            .seed_window(window_seed(&key_window))
            .await
            .expect("the key window seed")
    );
    let mut user_window = window(WINDOW, "5");
    user_window.counter_key = "spend:user:u1:window:1d".to_owned();
    user_window.entity_type = CounterEntityType::User;
    assert!(
        !backend
            .seed_window(window_seed(&user_window))
            .await
            .expect("the user window seed")
    );
}

// current_spend: the counter's own value once it holds one, the caller's fallback before
#[tokio::test]
async fn the_backend_reads_the_counter_before_its_fallback() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    assert_eq!(backend.current_spend(KEY, amount("7")).await, amount("7"));
    backend
        .increment_counter(KEY, amount("4"))
        .await
        .expect("the increment");
    assert_eq!(backend.current_spend(KEY, amount("7")).await, amount("4"));
}

// reserve_pipeline: the value every pending increment left in the memory tier
#[tokio::test]
async fn the_backend_pipeline_reports_the_counter_value_it_wrote() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    let pending = [PendingSpendIncrement {
        counter_key: KEY.to_owned(),
        increment: amount("4"),
    }];
    let counted = backend
        .reserve_pipeline(&pending)
        .await
        .expect("the reservation");
    assert_eq!(counted, vec![Some(amount("4"))]);
    let found = backend.read_cache_value(KEY).await.expect("the read");
    assert_eq!(found, Some(amount("4")));
}

// seed_source: an auth cache without a spend leaves the counter unwritten rather than storing zero
#[tokio::test]
async fn the_backend_skips_a_source_counter_without_a_spend() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    backend
        .seed_source(KEY, "k1")
        .await
        .expect("the source seed");
    let found = backend.read_cache_value(KEY).await.expect("the read");
    assert_eq!(found, None);
}

// SeedBatch: the wrapper reads and records through the request's own batch
#[tokio::test]
async fn the_seed_batch_reads_and_records_through_the_request_batch() {
    let seed_batch = SeedBatch(Arc::new(RecordingBatch::default()));
    seed_batch.record(KEY, amount("2"));
    let found = seed_batch.read(KEY).await.expect("the batch read");
    assert_eq!(found, Some(CounterValue::Amount(amount("2"))));
}
