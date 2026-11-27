#![cfg(test)]

use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::{TimeZone as _, Utc};
use rust_decimal::Decimal;

use super::refresh_orgs;
use crate::prometheus_budget_refresh::source::Page;
use crate::prometheus_budget_refresh::tests::support::{
    FakeSource, SetEvent, budget_rig, capture_logs, labels_source, now, org, prometheus_only,
    snapshot,
};

fn next_day() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 0, 0, 0).unwrap()
}

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_orgs() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["org_id", "org_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();

    refresh_orgs(
        None,
        &rig.gauges.org,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert!(rig.log.events().is_empty());
    assert!(
        logs.text()
            .contains("Prometheus: skipping org metrics initialization, DB not initialized"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn every_page_of_orgs_reaches_the_gauges() {
    let rig = budget_rig(&["org_id", "org_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_orgs(Ok(Page {
        rows: vec![org(
            "o-1",
            Some(Decimal::from(10)),
            Some(snapshot(Some(Decimal::from(50)), Some(next_day()))),
        )],
        total_count: Some(51),
    }));
    source.push_orgs(Ok(Page {
        rows: vec![org("o-2", None, None)],
        total_count: Some(51),
    }));

    refresh_orgs(
        Some(&source),
        &rig.gauges.org,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert_eq!(source.calls(), vec!["orgs:50:1", "orgs:50:2"]);
    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[0].value, MetricValue::Finite(Decimal::from(40)));
    assert_eq!(
        remaining[0].labels,
        vec![
            ("org_id".to_owned(), "o-1".to_owned()),
            ("org_alias".to_owned(), "o-1-alias".to_owned()),
        ]
    );
    assert_eq!(remaining[1].value, MetricValue::PositiveInfinity);
    assert_eq!(
        rig.log.for_metric("max"),
        vec![SetEvent {
            metric: "max".to_owned(),
            labels: vec![
                ("org_id".to_owned(), "o-1".to_owned()),
                ("org_alias".to_owned(), "o-1-alias".to_owned()),
            ],
            value: MetricValue::Finite(Decimal::from(50)),
        }]
    );
    let hours = rig.log.for_metric("hours");
    assert_eq!(hours.len(), 1);
    assert_eq!(hours[0].value, MetricValue::Finite(Decimal::from(24)));
}
