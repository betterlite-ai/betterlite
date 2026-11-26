use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::auth_model_budgets::rows::SpendReadError;
use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use betterlite_cache::coordinator::{CoordinatorError, EventDrivenCacheCoordinator};
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;

use crate::auth_model_budgets::global_spend::{
    EventGroup, GLOBAL_SPEND_LOG_PREFIX, GlobalSpendFetcher, ProxyBudgetAlert,
    ProxyBudgetAlertSink, ProxyBudgetRows, get_global_proxy_spend,
};

struct FakeRows {
    spend: Option<Decimal>,
    delay: Option<Duration>,
    gated: bool,
    started: Semaphore,
    release: Semaphore,
    calls: AtomicUsize,
}

impl FakeRows {
    fn new(spend: Option<Decimal>) -> Self {
        Self {
            spend,
            delay: None,
            gated: false,
            started: Semaphore::new(0),
            release: Semaphore::new(0),
            calls: AtomicUsize::new(0),
        }
    }

    fn gated(spend: Decimal) -> Self {
        Self {
            gated: true,
            ..Self::new(Some(spend))
        }
    }

    async fn wait_until_started(&self) {
        let _permit = timeout(Duration::from_secs(1), self.started.acquire())
            .await
            .expect("the leader starts its load")
            .expect("the semaphore stays open");
    }
}

#[async_trait]
impl ProxyBudgetRows for FakeRows {
    async fn find_proxy_budget(&self) -> Result<Option<Decimal>, SpendReadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.add_permits(1);
        if self.gated {
            let _release = self
                .release
                .acquire()
                .await
                .expect("the test releases the load");
        }
        if let Some(delay) = self.delay {
            sleep(delay).await;
        }
        Ok(self.spend)
    }
}

#[derive(Default)]
pub(super) struct FakeCache {
    stored: Mutex<Option<CacheValue>>,
    writes: Mutex<Vec<(String, CacheValue)>>,
}

#[async_trait]
impl CacheBackend for FakeCache {
    fn default_ttl(&self) -> i64 {
        60
    }
    async fn async_get_cache(&self, _key: &str) -> Result<Option<CacheValue>, BackendError> {
        Ok(self.stored.lock().unwrap().clone())
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        _ttl: Ttl,
    ) -> Result<(), BackendError> {
        self.writes
            .lock()
            .unwrap()
            .push((key.to_owned(), value.clone()));
        *self.stored.lock().unwrap() = Some(value);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Alerts {
    pub(super) scheduled: Mutex<Vec<ProxyBudgetAlert>>,
}

impl ProxyBudgetAlertSink for Alerts {
    fn schedule(&self, alert: ProxyBudgetAlert) {
        self.scheduled.lock().unwrap().push(alert);
    }
}

fn make_fetcher(
    rows: Arc<FakeRows>,
    cache: Arc<FakeCache>,
    alerts: Arc<Alerts>,
    deadline: f64,
    wait: Duration,
) -> GlobalSpendFetcher {
    GlobalSpendFetcher {
        rows,
        cache,
        coordinator: EventDrivenCacheCoordinator::new(GLOBAL_SPEND_LOG_PREFIX.to_owned(), wait),
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
        default_deadline_seconds: deadline,
        alerts,
    }
}

fn enabled_settings() -> Settings {
    Settings {
        max_budget: Decimal::new(50, 1),
    }
}

#[tokio::test]
async fn global_spend_singleflight() {
    let rows = Arc::new(FakeRows::gated(Decimal::new(125, 1)));
    let cache = Arc::new(FakeCache::default());
    let alerts = Arc::new(Alerts::default());
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::clone(&cache),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(5),
    );
    let settings = enabled_settings();
    let leader = get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key-a");
    let follower = async {
        rows.wait_until_started().await;
        rows.release.add_permits(1);
        get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key-b").await
    };
    let (first, second) = tokio::join!(leader, follower);
    let expected = Some(Decimal::new(125, 1));
    assert_eq!(first.unwrap(), expected);
    assert_eq!(second.unwrap(), expected);
    assert_eq!(rows.calls.load(Ordering::SeqCst), 1);
    let scheduled = alerts.scheduled.lock().unwrap().clone();
    assert_eq!(scheduled.len(), 2);
    let tokens: Vec<&str> = scheduled.iter().map(|alert| alert.token.as_str()).collect();
    assert_eq!(tokens, vec!["key-a", "key-b"]);
}

#[tokio::test]
async fn global_spend_exact_keys_and_bound() {
    let rows = Arc::new(FakeRows::new(Some(Decimal::new(35, 1))));
    let cache = Arc::new(FakeCache::default());
    let alerts = Arc::new(Alerts::default());
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::clone(&cache),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(5),
    );
    let settings = enabled_settings();
    let spend = get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key")
        .await
        .unwrap();
    assert_eq!(spend, Some(Decimal::new(35, 1)));
    let writes = cache.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, GLOBAL_PROXY_SPEND_CACHE_KEY);
    assert_eq!(writes[0].1, CacheValue::Text("3.5".to_owned()));
    assert_spend_deadline(&alerts).await;
}

async fn assert_spend_deadline(alerts: &Arc<Alerts>) {
    let settings = enabled_settings();
    // a read slower than the bound fails as a load error that names the lookup and the bound
    let slow = Arc::new(FakeRows {
        delay: Some(Duration::from_secs(5)),
        ..FakeRows::new(None)
    });
    let slow_cache = Arc::new(FakeCache::default());
    let slow_fetcher = make_fetcher(
        Arc::clone(&slow),
        Arc::clone(&slow_cache),
        Arc::clone(alerts),
        0.05,
        Duration::from_secs(5),
    );
    let error = get_global_proxy_spend(settings.max_budget, Some(&slow_fetcher), "admin", "key")
        .await
        .unwrap_err();
    let CoordinatorError::Load { key, source } = *error else {
        panic!("a slow global spend read is a load failure");
    };
    assert_eq!(key, GLOBAL_PROXY_SPEND_CACHE_KEY);
    let message = source.to_string();
    assert!(
        message.contains("proxy_budget lookup did not answer within 0.05s"),
        "{message}"
    );
    assert!(slow_cache.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn global_missing_row_no_alert() {
    let rows = Arc::new(FakeRows::new(None));
    let cache = Arc::new(FakeCache::default());
    let alerts = Arc::new(Alerts::default());
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::clone(&cache),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(5),
    );
    let settings = Settings::default();
    assert_eq!(
        get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        get_global_proxy_spend(settings.max_budget, None, "admin", "key")
            .await
            .unwrap(),
        None
    );
    assert_eq!(rows.calls.load(Ordering::SeqCst), 0);
    assert!(cache.writes.lock().unwrap().is_empty());
    assert!(alerts.scheduled.lock().unwrap().is_empty());
    // a positive ceiling reads the row once and caches the null it found
    let spend = get_global_proxy_spend(
        enabled_settings().max_budget,
        Some(&fetcher),
        "admin",
        "key",
    )
    .await
    .unwrap();
    assert_eq!(spend, None);
    assert_eq!(rows.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*cache.stored.lock().unwrap(), Some(CacheValue::Null));
    assert!(alerts.scheduled.lock().unwrap().is_empty());
}

#[tokio::test]
async fn global_zero_spend_alert_payload() {
    let rows = Arc::new(FakeRows::new(Some(Decimal::ZERO)));
    let cache = Arc::new(FakeCache::default());
    let alerts = Arc::new(Alerts::default());
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::clone(&cache),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(5),
    );
    let spend = get_global_proxy_spend(
        enabled_settings().max_budget,
        Some(&fetcher),
        "admin",
        "key",
    )
    .await
    .unwrap();
    assert_eq!(spend, Some(Decimal::ZERO));
    let scheduled = alerts.scheduled.lock().unwrap().clone();
    assert_eq!(
        scheduled,
        vec![ProxyBudgetAlert {
            admin_name: "admin".to_owned(),
            token: "key".to_owned(),
            max_budget: Decimal::new(50, 1),
            spend: Decimal::ZERO,
            event_group: EventGroup::Proxy,
        }]
    );
    assert_eq!(ProxyBudgetAlert::TYPE, "proxy_budget");
}

#[tokio::test]
async fn global_load_error_releases_waiters() {
    let rows = Arc::new(FakeRows::gated(Decimal::new(125, 1)));
    let cache = Arc::new(FakeCache::default());
    let alerts = Arc::new(Alerts::default());
    // the leader's read outlives its bound, and the waiter's own wait bound is longer
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::clone(&cache),
        Arc::clone(&alerts),
        0.05,
        Duration::from_secs(2),
    );
    let settings = enabled_settings();
    let leader = get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key-a");
    let follower = async {
        rows.wait_until_started().await;
        get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "key-b").await
    };
    let (first, second) = tokio::join!(leader, follower);
    let error = first.unwrap_err();
    assert!(matches!(*error, CoordinatorError::Load { .. }));
    // the failed load signals its waiters instead of leaving them to time out
    assert_eq!(second.unwrap(), None);
    assert!(alerts.scheduled.lock().unwrap().is_empty());
    assert!(cache.writes.lock().unwrap().is_empty());
}

#[rstest::rstest]
#[case::zero(CacheValue::Integer(0), Some(Decimal::ZERO))]
#[case::float(CacheValue::Float(1.5), Some(Decimal::new(15, 1)))]
#[case::text(CacheValue::Text("2.25".to_owned()), Some(Decimal::new(225, 2)))]
#[case::boolean(CacheValue::Boolean(false), None)]
#[case::invalid(CacheValue::Text("invalid".to_owned()), None)]
#[tokio::test]
async fn cached_numeric_spend_skips_database(
    #[case] value: CacheValue,
    #[case] expected: Option<Decimal>,
) {
    let rows = Arc::new(FakeRows::new(Some(Decimal::TEN)));
    let cache = Arc::new(FakeCache::default());
    *cache.stored.lock().unwrap() = Some(value);
    let alerts = Arc::new(Alerts::default());
    let fetcher = make_fetcher(
        Arc::clone(&rows),
        cache,
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(5),
    );
    let spend = get_global_proxy_spend(Decimal::ONE, Some(&fetcher), "owner", "key")
        .await
        .unwrap();
    assert_eq!(spend, expected);
    assert_eq!(rows.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        alerts.scheduled.lock().unwrap().len(),
        usize::from(expected.is_some())
    );
}

struct FailedWrite;

#[async_trait]
impl CacheBackend for FailedWrite {
    fn default_ttl(&self) -> i64 {
        60
    }
    async fn async_get_cache(&self, _key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }
    async fn async_set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Err(BackendError::Unsupported("failed-write"))
    }
}

#[tokio::test]
async fn cache_backend_failure_propagates_without_alert() {
    let rows = Arc::new(FakeRows::new(Some(Decimal::ONE)));
    let alerts = Arc::new(Alerts::default());
    let mut fetcher = make_fetcher(
        rows,
        Arc::new(FakeCache::default()),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(1),
    );
    fetcher.cache = Arc::new(FailedWrite);
    let error = get_global_proxy_spend(Decimal::TEN, Some(&fetcher), "admin", "key")
        .await
        .unwrap_err();
    assert!(matches!(
        *error,
        CoordinatorError::Cache {
            source: BackendError::Unsupported("failed-write"),
            ..
        }
    ));
    assert!(alerts.scheduled.lock().unwrap().is_empty());
}

struct FailedRead;

impl CacheBackend for FailedRead {
    fn default_ttl(&self) -> i64 {
        60
    }
}

#[tokio::test]
async fn cache_backend_read_failure_stops_database() {
    let rows = Arc::new(FakeRows::new(Some(Decimal::ONE)));
    let alerts = Arc::new(Alerts::default());
    let mut fetcher = make_fetcher(
        Arc::clone(&rows),
        Arc::new(FakeCache::default()),
        Arc::clone(&alerts),
        5.0,
        Duration::from_secs(1),
    );
    fetcher.cache = Arc::new(FailedRead);
    let error = get_global_proxy_spend(Decimal::TEN, Some(&fetcher), "admin", "key")
        .await
        .unwrap_err();
    assert!(matches!(*error, CoordinatorError::Cache { .. }));
    assert_eq!(rows.calls.load(Ordering::SeqCst), 0);
    assert!(alerts.scheduled.lock().unwrap().is_empty());
}
