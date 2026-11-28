#![cfg(test)]

use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::refresh_teams;
use crate::prometheus_budget_refresh::source::Page;
use crate::prometheus_budget_refresh::tests::support::{
    FakeSource, SetEvent, budget_rig, capture_logs, labels_source, now, prometheus_only, team,
};

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_teams() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["team", "team_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();

    refresh_teams(
        None,
        &rig.gauges.team,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert!(rig.log.events().is_empty());
    assert!(
        logs.text()
            .contains("Prometheus: skipping team metrics initialization, DB not initialized"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn every_page_of_teams_reaches_the_gauges() {
    let rig = budget_rig(&["team", "team_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_teams(Ok(Page {
        rows: vec![team(
            "alpha",
            Some("Alpha"),
            Some(Decimal::from(20_i32)),
            Some(Decimal::from(100_i32)),
        )],
        total_count: Some(51),
    }));
    source.push_teams(Ok(Page {
        rows: vec![team("bravo", None, None, None)],
        total_count: Some(51),
    }));

    let labels = labels_source(&settings, prometheus_only());
    refresh_teams(Some(&source), &rig.gauges.team, labels, now()).await;

    assert_eq!(source.calls(), vec!["teams:50:1", "teams:50:2"]);
    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 2);
    assert_eq!(
        remaining[0].value,
        MetricValue::Finite(Decimal::from(80_i32))
    );
    let expected = vec![
        ("team".to_owned(), "alpha".to_owned()),
        ("team_alias".to_owned(), "Alpha".to_owned()),
    ];
    assert_eq!(remaining[0].labels, expected);
    assert_eq!(remaining[1].value, MetricValue::PositiveInfinity);
    let max = rig.log.for_metric("max");
    assert_eq!(max.len(), 1);
    assert_eq!(max[0].value, MetricValue::Finite(Decimal::from(100_i32)));
}

#[tokio::test]
async fn a_team_without_a_limit_only_writes_the_remaining_gauge() {
    let rig = budget_rig(&[]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_teams(Ok(Page {
        rows: vec![team("no-limit", None, Some(Decimal::from(4_i32)), None)],
        total_count: Some(1),
    }));

    refresh_teams(
        Some(&source),
        &rig.gauges.team,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert_eq!(
        rig.log.events(),
        vec![SetEvent {
            metric: "remaining".to_owned(),
            labels: Vec::new(),
            value: MetricValue::PositiveInfinity,
        }]
    );
}
