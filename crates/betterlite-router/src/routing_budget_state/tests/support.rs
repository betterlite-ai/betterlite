use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tokio_util::task::TaskTracker;

use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::increment::BudgetIncrement;
use crate::budget_increment::queue::QueuedIncrement;
use crate::budget_increment::window::WindowRedis;
use crate::budget_windows::budget::{BudgetPeriod, DeploymentLimits, ProviderBudget, TagBudget};
use crate::budget_windows::service::BudgetWindows;
use crate::routing_budget_state::error::BudgetStateError;
use crate::routing_budget_state::event::{CallType, SuccessEvent};
use crate::routing_budget_state::service::RoutingBudgetState;
use crate::routing_budget_state::spend_redis::{SpendRead, SpendRedis};
use crate::routing_value_store::Clock;

pub(super) const DAY_SECONDS: u64 = 86_400;
pub(super) const PROVIDER_KEY: &str = "provider_spend:openai:1d";
pub(super) const DEPLOYMENT_KEY: &str = "deployment_spend:dep-1:2h";
pub(super) const TAG_KEY: &str = "tag_spend:team-a:1h";
pub(super) const DEFAULT_TTL_SECONDS: f64 = 600.0;

#[derive(Clone, Default)]
pub(super) struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub(super) fn new(now: f64) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    pub(super) fn set(&self, now: f64) {
        *self.0.lock().unwrap() = now;
    }

    pub(super) fn clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap())
    }
}

// the shared store the limiter writes to; it keeps the decimal a test sent, so a sum reads back exactly
#[derive(Clone, Default)]
pub(super) struct FakeBudgetRedis {
    spends: Arc<Mutex<BTreeMap<String, Decimal>>>,
    starts: Arc<Mutex<BTreeMap<String, f64>>>,
    read_keys: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    fail_reads: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl FakeBudgetRedis {
    pub(super) fn spend(&self, key: &str) -> Option<Decimal> {
        self.spends.lock().unwrap().get(key).copied()
    }

    pub(super) fn seed_spend(&self, key: &str, spend: Decimal) {
        self.spends.lock().unwrap().insert(key.to_owned(), spend);
    }

    pub(super) fn read_keys(&self) -> Vec<String> {
        self.read_keys.lock().unwrap().clone()
    }

    pub(super) fn start_time(&self, key: &str) -> Option<f64> {
        self.starts.lock().unwrap().get(key).copied()
    }

    pub(super) fn fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    pub(super) fn hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    pub(super) fn fail_reads(&self, fail: bool) {
        self.fail_reads.store(fail, Ordering::SeqCst);
    }

    pub(super) fn entered(&self) -> &Notify {
        &self.entered
    }

    pub(super) fn release(&self) {
        self.release.notify_one();
    }
}

#[async_trait]
impl WindowRedis for FakeBudgetRedis {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        Ok(self.starts.lock().unwrap().get(key).copied())
    }

    async fn write_start_time(&self, key: &str, start_time: f64, _ttl: Duration) {
        self.starts
            .lock()
            .unwrap()
            .insert(key.to_owned(), start_time);
    }

    async fn write_spend(&self, key: &str, cost: Decimal, _ttl: Duration) {
        self.spends.lock().unwrap().insert(key.to_owned(), cost);
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        if self.hold.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return false;
        }
        let mut spends = self.spends.lock().unwrap();
        let mut working = spends.clone();
        for operation in operations {
            let current = working.get(&operation.key).copied().unwrap_or_default();
            let Some(total) = current.checked_add(operation.increment_value) else {
                return false;
            };
            working.insert(operation.key.clone(), total);
        }
        *spends = working;
        true
    }
}

#[async_trait]
impl SpendRedis for FakeBudgetRedis {
    async fn read_spends(&self, keys: &[String]) -> SpendRead {
        if self.fail_reads.load(Ordering::SeqCst) {
            return SpendRead::Failed;
        }
        let mut recorded = self.read_keys.lock().unwrap();
        recorded.extend(keys.iter().cloned());
        drop(recorded);
        let spends = self.spends.lock().unwrap();
        SpendRead::Values(
            keys.iter()
                .map(|key| (key.clone(), spends.get(key).copied()))
                .collect(),
        )
    }
}

pub(super) struct Harness {
    pub(super) state: Arc<RoutingBudgetState<FakeBudgetRedis>>,
    pub(super) memory: Arc<Mutex<LocalTtlCache>>,
    store: Option<FakeBudgetRedis>,
}

impl Harness {
    pub(super) fn store(&self) -> &FakeBudgetRedis {
        self.store.as_ref().unwrap()
    }

    pub(super) fn expiry(&self, key: &str) -> Option<f64> {
        self.memory
            .lock()
            .unwrap()
            .ttl_entries()
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, expiry)| expiry)
    }

    pub(super) fn spend(&self, key: &str) -> Option<Decimal> {
        let value = self.memory.lock().unwrap().get(key).unwrap()?;
        let CacheValue::Object(fields) = value else {
            return None;
        };
        let CacheValue::Text(text) = fields.get("spend")? else {
            return None;
        };
        Decimal::from_str(text).ok()
    }

    pub(super) fn queued(&self) -> Vec<(String, Decimal)> {
        self.state
            .increments
            .queue
            .lock()
            .unwrap()
            .queued
            .iter()
            .map(|operation| (operation.key.clone(), operation.increment_value))
            .collect()
    }
}

// one rejected increment for the provider window, so a test can read the error the tiers report
pub(super) async fn spend_error(harness: &Harness) -> BudgetStateError {
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("1")))
        .await
        .unwrap_err()
}

pub(super) fn harness(windows: BudgetWindows) -> Harness {
    harness_at(windows, &TestClock::new(0.0))
}

pub(super) fn harness_at(windows: BudgetWindows, clock: &TestClock) -> Harness {
    harness_with(windows, Some(FakeBudgetRedis::default()), clock)
}

pub(super) fn harness_without_store(windows: BudgetWindows) -> Harness {
    harness_with(windows, None, &TestClock::new(0.0))
}

fn harness_with(
    windows: BudgetWindows,
    store: Option<FakeBudgetRedis>,
    clock: &TestClock,
) -> Harness {
    let memory = Arc::new(Mutex::new(
        LocalTtlCache::with_clock(
            Some(200),
            Some(DEFAULT_TTL_SECONDS),
            None,
            1024,
            clock.clock(),
        )
        .unwrap(),
    ));
    let cache = Arc::new(DualWindowCache {
        memory: Arc::clone(&memory),
        redis: store.clone(),
    });
    Harness {
        state: Arc::new(RoutingBudgetState {
            increments: BudgetIncrement {
                cache,
                clock: clock.clock(),
                queue: Arc::default(),
                flush_lock: Arc::default(),
            },
            windows,
            sync: Mutex::new(JoinSet::new()),
            flushes: TaskTracker::new(),
        }),
        memory,
        store,
    }
}

pub(super) fn all_windows() -> BudgetWindows {
    BudgetWindows::new(
        Some(providers(Some(period("1d", DAY_SECONDS)))),
        Some(tags()),
        &[deployment()],
    )
}

pub(super) fn provider_without_a_period() -> BudgetWindows {
    BudgetWindows::new(Some(providers(None)), None, &[])
}

pub(super) fn providers(at: Option<BudgetPeriod>) -> IndexMap<String, ProviderBudget> {
    IndexMap::from([(
        "openai".to_owned(),
        ProviderBudget {
            budget_limit: Some(Decimal::from(10_u8)),
            period: at,
        },
    )])
}

pub(super) fn tags() -> IndexMap<String, TagBudget> {
    IndexMap::from([(
        "team-a".to_owned(),
        TagBudget {
            max_budget: Some(Decimal::from(5_u8)),
            period: Some(period("1h", 3_600)),
        },
    )])
}

pub(super) fn deployment() -> DeploymentLimits {
    DeploymentLimits {
        model_id: Some("dep-1".to_owned()),
        max_budget: Some(Decimal::from(20_u8)),
        period: Some(period("2h", 7_200)),
    }
}

pub(super) fn period(spec: &str, seconds: u64) -> BudgetPeriod {
    BudgetPeriod {
        spec: spec.to_owned(),
        seconds: Duration::from_secs(seconds),
    }
}

pub(super) fn cost(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(super) fn event<'event>(
    provider: Option<&'event str>,
    model_id: Option<&'event str>,
    tags: &'event [String],
    response_cost: Decimal,
) -> SuccessEvent<'event> {
    SuccessEvent {
        call_type: CallType::parse(None),
        response_cost,
        model_id,
        custom_llm_provider: provider,
        tags,
    }
}

// 32 loggers against 16 syncs: the final sync runs after every logger, so both tiers hold the full sum
pub(super) async fn concurrent_syncs() {
    let harness = harness(all_windows());
    let mut loggers = JoinSet::new();
    for _worker in 0..32_u32 {
        let _task = loggers.spawn(log_costs(Arc::clone(&harness.state)));
    }
    let mut syncs = JoinSet::new();
    let _task = syncs.spawn(sync_rounds(Arc::clone(&harness.state)));
    while loggers.join_next().await.is_some() {}
    while syncs.join_next().await.is_some() {}
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("2.56")));
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("2.56")));
    assert!(harness.queued().is_empty());
}

async fn log_costs(state: Arc<RoutingBudgetState<FakeBudgetRedis>>) {
    let tags: [String; 0] = [];
    for _step in 0..8_u32 {
        state
            .log_success(&event(Some("openai"), None, &tags, cost("0.01")))
            .await
            .unwrap();
    }
}

async fn sync_rounds(state: Arc<RoutingBudgetState<FakeBudgetRedis>>) {
    for _round in 0..16_u32 {
        let _synced = state.sync_with_redis().await;
        yield_now().await;
    }
}
