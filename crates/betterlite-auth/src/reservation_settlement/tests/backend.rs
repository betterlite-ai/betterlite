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
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::reservation_settlement::backend::SpendCounterBackend;
use crate::reservation_settlement::counters::SettlementCounters as _;
use crate::reservation_settlement::release::release_budget_reservation;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::source::SourceSpendCache;

use super::support::{decimal, entry, handle};

#[derive(Debug, thiserror::Error)]
#[error("the fake spend row read failed")]
struct RowsError;

struct FakeRows {
    spend: Option<Decimal>,
}

impl DbSpendCounterRows for FakeRows {
    type Error = RowsError;

    fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<DbSpendRow>, RowsError>> + Send {
        ready(Ok(self
            .spend
            .map(|spend| DbSpendRow { spend: Some(spend) })))
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
        Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0)
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

struct NoSources;

#[async_trait]
impl SourceSpendCache for NoSources {
    async fn cached(&self, _cache_key: &str) -> Option<Json> {
        None
    }
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
    spend: Option<&str>,
) -> (
    SpendCounterWrites<DualCache<LocalMemoryOps>, FakeRows>,
    Arc<SpendCounterScope>,
) {
    let scope = Arc::new(SpendCounterScope::default());
    let batch: Arc<dyn CounterBatch> = Arc::clone(&scope);
    let sources: Arc<dyn SourceSpendCache> = Arc::new(NoSources);
    let reseed = Arc::new(SpendCounterReseed {
        rows: Some(Arc::new(FakeRows {
            spend: spend.map(decimal),
        })),
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

#[tokio::test]
async fn the_backend_reads_and_increments_the_shared_memory_tier() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    let found = backend
        .read_cache_value("spend:key:k1")
        .await
        .expect("the read");
    assert_eq!(found, None);
    backend
        .increment_counter("spend:key:k1", decimal("4"))
        .await
        .expect("the increment");
    let found = backend
        .read_cache_value("spend:key:k1")
        .await
        .expect("the read");
    assert_eq!(found, Some(decimal("4")));
}

#[tokio::test]
async fn a_database_floor_is_reseeded_into_the_counter() {
    let (writes, scope) = backend_fixture(Some("7"));
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    assert!(backend.reseed_from_db("spend:key:k1").await);
    let found = backend
        .read_cache_value("spend:key:k1")
        .await
        .expect("the read");
    assert_eq!(found, Some(decimal("7")));
}

#[tokio::test]
async fn invalidation_clears_the_counter_from_the_memory_tier() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    backend
        .increment_counter("spend:key:k1", decimal("4"))
        .await
        .expect("the increment");
    backend.invalidate("spend:key:k1").await;
    let found = backend
        .read_cache_value("spend:key:k1")
        .await
        .expect("the read");
    assert_eq!(found, None);
}

#[tokio::test]
async fn a_settle_through_the_backend_moves_the_real_counter_to_the_actual_cost() {
    let (writes, scope) = backend_fixture(None);
    let backend = SpendCounterBackend {
        writes: &writes,
        scope: scope.as_ref(),
        redis: None,
    };
    backend
        .increment_counter("spend:key:k1", decimal("10"))
        .await
        .expect("the reservation");
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    release_budget_reservation(&backend, &reservation)
        .await
        .expect("the release");
    let found = backend
        .read_cache_value("spend:key:k1")
        .await
        .expect("the read");
    assert_eq!(found, Some(Decimal::ZERO));
}
