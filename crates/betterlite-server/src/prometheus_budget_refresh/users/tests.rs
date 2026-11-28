#![cfg(test)]

use betterlite_logging::prometheus::budget_gauges::inputs::UserBudgetGaugeInput;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::refresh_users;
use crate::prometheus_budget_refresh::source::Page;
use crate::prometheus_budget_refresh::tests::support::{
    FakeSource, budget_rig, capture_logs, labels_source, now, prometheus_only,
};

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_users() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["user"]);
    let settings = PrometheusLabelSettings::with_defaults();

    refresh_users(
        None,
        &rig.gauges.user,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert!(rig.log.events().is_empty());
    assert!(
        logs.text()
            .contains("Prometheus: skipping user metrics initialization, DB not initialized"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn every_page_of_users_reaches_the_gauges() {
    let rig = budget_rig(&["user"]);
    let settings = PrometheusLabelSettings::with_defaults();
    let source = FakeSource::new();
    source.push_users(Ok(Page {
        rows: vec![UserBudgetGaugeInput {
            user_id: "u-1".to_owned(),
            user_email: None,
            user_alias: None,
            spend: Decimal::from(30_i32),
            max_budget: Some(Decimal::from(100_i32)),
            budget_reset_at: None,
        }],
        total_count: Some(51),
    }));
    source.push_users(Ok(Page {
        rows: vec![UserBudgetGaugeInput {
            user_id: "u-2".to_owned(),
            user_email: None,
            user_alias: None,
            spend: Decimal::from(5_i32),
            max_budget: None,
            budget_reset_at: None,
        }],
        total_count: Some(51),
    }));

    refresh_users(
        Some(&source),
        &rig.gauges.user,
        labels_source(&settings, prometheus_only()),
        now(),
    )
    .await;

    assert_eq!(source.calls(), vec!["users:50:1", "users:50:2"]);
    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 2);
    assert_eq!(
        remaining[0].value,
        MetricValue::Finite(Decimal::from(70_i32))
    );
    assert_eq!(
        remaining[0].labels,
        vec![("user".to_owned(), "u-1".to_owned())]
    );
    assert_eq!(remaining[1].value, MetricValue::PositiveInfinity);
    let max = rig.log.for_metric("max");
    assert_eq!(max.len(), 1);
    assert_eq!(max[0].value, MetricValue::Finite(Decimal::from(100_i32)));
}
