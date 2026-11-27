#![cfg(test)]

use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricLike;

use super::{BudgetRefresh, BudgetRefreshLock, initialize_remaining_budget_metrics};
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
