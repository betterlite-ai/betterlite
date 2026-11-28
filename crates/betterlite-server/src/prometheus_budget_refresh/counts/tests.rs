#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::{MetricLike, MetricValue};
use rust_decimal::Decimal;

use super::{CountGauges, refresh_counts};
use crate::prometheus_budget_refresh::source::BudgetPageError;
use crate::prometheus_budget_refresh::tests::support::{
    BudgetRig, FakeSource, ProbeGauge, budget_rig, capture_logs, real_gauge,
};

const fn window_error() -> BudgetPageError {
    BudgetPageError::Window {
        page: 1,
        page_size: 50,
    }
}

struct CountSetup {
    rig: BudgetRig,
    total_users: MetricLike<ProbeGauge>,
    active_users: MetricLike<ProbeGauge>,
    teams: MetricLike<ProbeGauge>,
}

fn count_setup() -> CountSetup {
    let rig = budget_rig(&[]);
    CountSetup {
        total_users: real_gauge("total_users", &rig.log),
        active_users: real_gauge("active_users", &rig.log),
        teams: real_gauge("team_count", &rig.log),
        rig,
    }
}

impl CountSetup {
    const fn gauges(&self) -> CountGauges<'_, MetricLike<ProbeGauge>> {
        CountGauges {
            total_users: &self.total_users,
            active_users: &self.active_users,
            teams: &self.teams,
        }
    }
}

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_counts() {
    let (logs, _guard) = capture_logs();
    let setup = count_setup();

    refresh_counts(None, &setup.gauges()).await;

    assert!(setup.rig.log.events().is_empty());
    assert!(
        logs.text().contains(
            "Prometheus: skipping user/team count metrics initialization, DB not initialized"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn the_three_counts_are_written_in_order() {
    let (logs, _guard) = capture_logs();
    let setup = count_setup();
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Ok(3));
    source.push_count(Ok(7));

    refresh_counts(Some(&source), &setup.gauges()).await;

    assert_eq!(
        source.calls(),
        vec!["user_count", "billable_user_count", "team_count"]
    );
    for (metric, value) in [
        ("total_users", 5_i32),
        ("active_users", 3),
        ("team_count", 7),
    ] {
        assert_eq!(
            setup.rig.log.for_metric(metric)[0].value,
            MetricValue::Finite(Decimal::from(value))
        );
    }
    let captured = logs.text();
    for message in [
        "Prometheus: set litellm_total_users to 5",
        "Prometheus: set litellm_active_users to 3",
        "Prometheus: set litellm_teams_count to 7",
    ] {
        assert!(captured.contains(message), "{captured}");
    }
}

#[tokio::test]
async fn the_first_failed_count_stops_the_rest() {
    let (logs, _guard) = capture_logs();
    let setup = count_setup();
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Err(window_error()));
    source.push_count(Ok(7));

    refresh_counts(Some(&source), &setup.gauges()).await;

    assert_eq!(source.calls(), vec!["user_count", "billable_user_count"]);
    assert_eq!(
        setup.rig.log.for_metric("total_users")[0].value,
        MetricValue::Finite(Decimal::from(5_i32))
    );
    assert!(setup.rig.log.for_metric("active_users").is_empty());
    assert!(setup.rig.log.for_metric("team_count").is_empty());
    assert!(
        logs.text().contains(
            "Error initializing user/team count metrics: budget page 1 with page size 50 has no database offset"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_failed_team_count_keeps_the_two_user_counts() {
    let (logs, _guard) = capture_logs();
    let setup = count_setup();
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Ok(3));
    source.push_count(Err(window_error()));

    refresh_counts(Some(&source), &setup.gauges()).await;

    assert_eq!(setup.rig.log.for_metric("total_users").len(), 1);
    assert_eq!(setup.rig.log.for_metric("active_users").len(), 1);
    assert!(setup.rig.log.for_metric("team_count").is_empty());
    assert!(
        logs.text()
            .contains("Error initializing user/team count metrics:"),
        "{}",
        logs.text()
    );
}
