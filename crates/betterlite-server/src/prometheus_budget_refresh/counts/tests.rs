#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::{CountGauges, refresh_counts};
use crate::prometheus_budget_refresh::source::BudgetPageError;
use crate::prometheus_budget_refresh::tests::support::{
    FakeSource, budget_rig, capture_logs, real_gauge,
};

const fn window_error() -> BudgetPageError {
    BudgetPageError::Window {
        page: 1,
        page_size: 50,
    }
}

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_counts() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&[]);
    let total_users = real_gauge("total_users", &rig.log);
    let active_users = real_gauge("active_users", &rig.log);
    let teams = real_gauge("team_count", &rig.log);
    let gauges = CountGauges {
        total_users: &total_users,
        active_users: &active_users,
        teams: &teams,
    };

    refresh_counts(None, &gauges).await;

    assert!(rig.log.events().is_empty());
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
    let rig = budget_rig(&[]);
    let total_users = real_gauge("total_users", &rig.log);
    let active_users = real_gauge("active_users", &rig.log);
    let teams = real_gauge("team_count", &rig.log);
    let gauges = CountGauges {
        total_users: &total_users,
        active_users: &active_users,
        teams: &teams,
    };
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Ok(3));
    source.push_count(Ok(7));

    refresh_counts(Some(&source), &gauges).await;

    assert_eq!(
        source.calls(),
        vec!["user_count", "billable_user_count", "team_count"]
    );
    assert_eq!(
        rig.log.for_metric("total_users")[0].value,
        MetricValue::Finite(Decimal::from(5))
    );
    assert_eq!(
        rig.log.for_metric("active_users")[0].value,
        MetricValue::Finite(Decimal::from(3))
    );
    assert_eq!(
        rig.log.for_metric("team_count")[0].value,
        MetricValue::Finite(Decimal::from(7))
    );
    let captured = logs.text();
    assert!(
        captured.contains("Prometheus: set litellm_total_users to 5"),
        "{captured}"
    );
    assert!(
        captured.contains("Prometheus: set litellm_active_users to 3"),
        "{captured}"
    );
    assert!(
        captured.contains("Prometheus: set litellm_teams_count to 7"),
        "{captured}"
    );
}

#[tokio::test]
async fn the_first_failed_count_stops_the_rest() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&[]);
    let total_users = real_gauge("total_users", &rig.log);
    let active_users = real_gauge("active_users", &rig.log);
    let teams = real_gauge("team_count", &rig.log);
    let gauges = CountGauges {
        total_users: &total_users,
        active_users: &active_users,
        teams: &teams,
    };
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Err(window_error()));
    source.push_count(Ok(7));

    refresh_counts(Some(&source), &gauges).await;

    assert_eq!(source.calls(), vec!["user_count", "billable_user_count"]);
    assert_eq!(
        rig.log.for_metric("total_users")[0].value,
        MetricValue::Finite(Decimal::from(5))
    );
    assert!(rig.log.for_metric("active_users").is_empty());
    assert!(rig.log.for_metric("team_count").is_empty());
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
    let rig = budget_rig(&[]);
    let total_users = real_gauge("total_users", &rig.log);
    let active_users = real_gauge("active_users", &rig.log);
    let teams = real_gauge("team_count", &rig.log);
    let gauges = CountGauges {
        total_users: &total_users,
        active_users: &active_users,
        teams: &teams,
    };
    let source = FakeSource::new();
    source.push_count(Ok(5));
    source.push_count(Ok(3));
    source.push_count(Err(window_error()));

    refresh_counts(Some(&source), &gauges).await;

    assert_eq!(rig.log.for_metric("total_users").len(), 1);
    assert_eq!(rig.log.for_metric("active_users").len(), 1);
    assert!(rig.log.for_metric("team_count").is_empty());
    assert!(
        logs.text()
            .contains("Error initializing user/team count metrics:"),
        "{}",
        logs.text()
    );
}
