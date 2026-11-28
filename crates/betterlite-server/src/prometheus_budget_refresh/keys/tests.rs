#![cfg(test)]

use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::refresh_keys;
use crate::prometheus_budget_refresh::source::Page;
use crate::prometheus_budget_refresh::tests::support::{
    FakeSource, auth_entry, budget_rig, capture_logs, deleted_entry, labels_source, now,
    prometheus_only, token_entry,
};

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_keys() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&[]);
    let settings = PrometheusLabelSettings::with_defaults();

    refresh_keys(
        None,
        &rig.gauges.key,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert!(rig.log.events().is_empty());
    assert!(
        logs.text()
            .contains("Prometheus: skipping key metrics initialization, DB not initialized"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn token_strings_and_deleted_records_are_skipped() {
    let rig = budget_rig(&["hashed_api_key", "api_key_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_keys(Ok(Page {
        rows: vec![token_entry("raw-key"), deleted_entry("gone-key")],
        total_count: Some(2),
    }));

    refresh_keys(
        Some(&source),
        &rig.gauges.key,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert_eq!(source.calls(), vec!["keys:50:1"]);
    assert!(rig.log.events().is_empty());
}

#[tokio::test]
async fn auth_records_reach_the_gauges_with_their_labels() {
    let rig = budget_rig(&["hashed_api_key", "api_key_alias"]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_keys(Ok(Page {
        rows: vec![auth_entry(
            "hashed-key",
            "prod",
            Decimal::from(10_i32),
            Some(Decimal::from(25_i32)),
        )],
        total_count: Some(1),
    }));

    refresh_keys(
        Some(&source),
        &rig.gauges.key,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].value,
        MetricValue::Finite(Decimal::from(15_i32))
    );
    assert_eq!(
        remaining[0].labels,
        vec![
            ("api_key_alias".to_owned(), "prod".to_owned()),
            ("hashed_api_key".to_owned(), "hashed-key".to_owned()),
        ]
    );
    let max = rig.log.for_metric("max");
    assert_eq!(max.len(), 1);
    assert_eq!(max[0].value, MetricValue::Finite(Decimal::from(25_i32)));
}

#[tokio::test]
async fn every_page_of_keys_reaches_the_gauges() {
    let rig = budget_rig(&[]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_keys(Ok(Page {
        rows: vec![auth_entry("hashed-one", "one", Decimal::from(1_i32), None)],
        total_count: Some(51),
    }));
    source.push_keys(Ok(Page {
        rows: vec![auth_entry(
            "hashed-two",
            "two",
            Decimal::from(2_i32),
            Some(Decimal::from(9_i32)),
        )],
        total_count: Some(51),
    }));

    refresh_keys(
        Some(&source),
        &rig.gauges.key,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert_eq!(source.calls(), vec!["keys:50:1", "keys:50:2"]);
    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[0].value, MetricValue::PositiveInfinity);
    assert_eq!(
        remaining[1].value,
        MetricValue::Finite(Decimal::from(7_i32))
    );
}
