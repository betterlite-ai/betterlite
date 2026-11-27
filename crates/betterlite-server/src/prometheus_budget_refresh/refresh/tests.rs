#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricLike;
use uuid::Uuid;

use super::{BudgetRefresh, BudgetRefreshLock, LOCK_JOB_NAME, initialize_remaining_budget_metrics};
use crate::prometheus_budget_refresh::counts::CountGauges;
use crate::prometheus_budget_refresh::source::BudgetPageError;
use crate::prometheus_budget_refresh::tests::support::{
    BudgetRig, FakeLock, FakeLookups, FakeSource, ProbeGauge, budget_rig, capture_logs, clock,
    no_limits, prometheus_only, real_gauge,
};

struct Harness {
    rig: BudgetRig,
    source: FakeSource,
    lookups: FakeLookups,
    labels: PrometheusLabelSettings,
    tracker: BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
    total_users: MetricLike<ProbeGauge>,
    active_users: MetricLike<ProbeGauge>,
    team_gauge: MetricLike<ProbeGauge>,
}

fn harness() -> Harness {
    let rig = budget_rig(&[]);
    let total_users = real_gauge("total_users", &rig.log);
    let active_users = real_gauge("active_users", &rig.log);
    let team_gauge = real_gauge("team_count", &rig.log);
    Harness {
        rig,
        source: FakeSource::new(),
        lookups: FakeLookups::with_budget(None),
        labels: PrometheusLabelSettings::with_defaults(),
        tracker: BoundedPrometheusSeriesTracker::default(),
        limits: no_limits(),
        total_users,
        active_users,
        team_gauge,
    }
}

impl Harness {
    async fn run(&self, lock: Option<&dyn BudgetRefreshLock>) {
        let refresh = BudgetRefresh {
            gauges: &self.rig.gauges,
            counts: CountGauges {
                total_users: &self.total_users,
                active_users: &self.active_users,
                teams: &self.team_gauge,
            },
            source: Some(&self.source),
            lookups: &self.lookups,
            default_budget_id: None,
            labels: &self.labels,
            end_user_settings: prometheus_only(),
            tracker: &self.tracker,
            limits: &self.limits,
            clock: clock(),
            lock,
        };
        initialize_remaining_budget_metrics(&refresh).await;
    }
}

const fn window_error() -> BudgetPageError {
    BudgetPageError::Window {
        page: 1,
        page_size: 50,
    }
}

const ALL_ENTITIES: [&str; 8] = [
    "teams:50:1",
    "keys:50:1",
    "users:50:1",
    "orgs:50:1",
    "customers:50:1:true",
    "user_count",
    "billable_user_count",
    "team_count",
];

#[tokio::test]
async fn a_refresh_without_a_lock_reaches_every_entity() {
    let (logs, _guard) = capture_logs();
    let harness = harness();

    harness.run(None).await;

    assert_eq!(harness.source.calls(), ALL_ENTITIES.to_vec());
    assert!(
        logs.text()
            .contains("Emitting key, team, user, org, customer budget metrics...."),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_lock_without_a_cache_runs_without_acquiring() {
    let harness = harness();
    let lock = FakeLock::without_cache();

    harness.run(Some(&lock)).await;

    assert!(lock.calls().is_empty());
    assert_eq!(harness.source.calls(), ALL_ENTITIES.to_vec());
}

#[tokio::test]
async fn a_denied_lock_skips_every_entity() {
    let (logs, _guard) = capture_logs();
    let harness = harness();
    let lock = FakeLock::with_cache(Some(false));

    harness.run(Some(&lock)).await;

    assert_eq!(
        lock.calls(),
        vec!["acquire:prometheus_emit_budget_metrics".to_owned()]
    );
    assert!(harness.source.calls().is_empty());
    assert!(!logs.text().contains("Emitting key, team"));
}

#[tokio::test]
async fn a_lock_answer_of_none_skips_every_entity() {
    let harness = harness();
    let lock = FakeLock::with_cache(None);

    harness.run(Some(&lock)).await;

    assert_eq!(
        lock.calls(),
        vec!["acquire:prometheus_emit_budget_metrics".to_owned()]
    );
    assert!(harness.source.calls().is_empty());
}

#[tokio::test]
async fn an_acquired_lock_runs_every_entity_then_releases() {
    let harness = harness();
    let lock = FakeLock::with_cache(Some(true));

    harness.run(Some(&lock)).await;

    assert_eq!(
        lock.calls(),
        vec![
            "acquire:prometheus_emit_budget_metrics".to_owned(),
            "release:prometheus_emit_budget_metrics".to_owned(),
        ]
    );
    assert_eq!(harness.source.calls(), ALL_ENTITIES.to_vec());
}

#[tokio::test]
async fn a_failing_entity_does_not_stop_the_refresh() {
    let (logs, _guard) = capture_logs();
    let harness = harness();
    harness.source.push_teams(Err(window_error()));

    harness.run(None).await;

    assert_eq!(harness.source.calls(), ALL_ENTITIES.to_vec());
    assert!(
        logs.text().contains(
            "Error initializing teams budget metrics: budget page 1 with page size 50 has no database offset"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_lock_is_released_after_a_failing_entity() {
    let (logs, _guard) = capture_logs();
    let harness = harness();
    harness.source.push_teams(Err(window_error()));
    let lock = FakeLock::with_cache(Some(true));

    harness.run(Some(&lock)).await;

    assert_eq!(
        lock.calls(),
        vec![
            "acquire:prometheus_emit_budget_metrics".to_owned(),
            "release:prometheus_emit_budget_metrics".to_owned(),
        ]
    );
    assert!(
        logs.text()
            .contains("Error initializing teams budget metrics:"),
        "{}",
        logs.text()
    );
}

// A lease store the tests control: free means the next acquire wins, otherwise the stored owner keeps it.
struct FakeCache {
    free: bool,
    owner: Mutex<Option<String>>,
    deletes: Mutex<Vec<String>>,
}

impl FakeCache {
    fn free() -> Self {
        Self {
            free: true,
            owner: Mutex::new(None),
            deletes: Mutex::new(Vec::new()),
        }
    }

    fn held_by(owner: &str) -> Self {
        Self {
            free: false,
            owner: Mutex::new(Some(owner.to_owned())),
            deletes: Mutex::new(Vec::new()),
        }
    }

    fn deletes(&self) -> Vec<String> {
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LockCache for FakeCache {
    async fn get_owner(&self, _key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        Ok(self
            .owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .map(String::into_bytes))
    }

    async fn set_owner_if_absent(
        &self,
        _key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        if self.free {
            *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = Some(owner.to_owned());
        }
        Ok(self.free)
    }

    async fn delete(&self, key: &str) -> Result<i64, LockCacheError> {
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        Ok(1)
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn pod_id() -> PodId {
    PodId::try_from(Uuid::new_v4()).unwrap()
}

fn manager(cache: Arc<FakeCache>) -> PodLockManager<FakeCache> {
    PodLockManager::new(Some(cache), 60, Arc::new(NoPodLockEventSink), pod_id())
}

#[tokio::test]
async fn the_pod_lock_wraps_a_cache_that_grants_the_lease() {
    let cache = Arc::new(FakeCache::free());
    let manager = manager(cache.clone());

    assert!(BudgetRefreshLock::has_cache(&manager));
    assert_eq!(
        BudgetRefreshLock::acquire_lock(&manager, LOCK_JOB_NAME).await,
        Some(true)
    );
    BudgetRefreshLock::release_lock(&manager, LOCK_JOB_NAME).await;
    assert_eq!(
        cache.deletes(),
        vec![format!("cronjob_lock:{LOCK_JOB_NAME}")]
    );
}

#[tokio::test]
async fn a_pod_lock_without_a_cache_does_nothing() {
    let manager: PodLockManager<FakeCache> =
        PodLockManager::new(None, 60, Arc::new(NoPodLockEventSink), pod_id());

    assert!(!BudgetRefreshLock::has_cache(&manager));
    assert_eq!(
        BudgetRefreshLock::acquire_lock(&manager, LOCK_JOB_NAME).await,
        None
    );
    BudgetRefreshLock::release_lock(&manager, LOCK_JOB_NAME).await;
}

#[tokio::test]
async fn a_pod_lock_held_by_another_pod_is_not_granted() {
    let manager = manager(Arc::new(FakeCache::held_by("another-pod")));

    assert_eq!(
        BudgetRefreshLock::acquire_lock(&manager, LOCK_JOB_NAME).await,
        Some(false)
    );
}
