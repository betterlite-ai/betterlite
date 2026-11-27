#![cfg(test)]

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::BoundedPrometheusSeriesTracker;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::{MetricLike, MetricValue};
use rust_decimal::Decimal;

use super::{CustomerRefresh, refresh_customers};
use crate::prometheus_budget_refresh::source::{BudgetPageSource, Page};
use crate::prometheus_budget_refresh::tests::support::{
    FakeLookups, FakeSource, ProbeGauge, budget_rig, capture_logs, clock, customer,
    customer_context, end_user_tracking_disabled, labels_source, no_limits, noop_entity,
    prometheus_only, prometheus_only_off, snapshot,
};

async fn refresh(
    source: Option<&FakeSource>,
    lookups: &FakeLookups,
    default_budget_id: Option<&str>,
    gauges: &EntityBudgetGauges<MetricLike<ProbeGauge>>,
    settings: EndUserCostTrackingSettings,
) {
    let labels = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = no_limits();
    let context = customer_context(labels_source(&labels, settings), clock(), &tracker, &limits);
    let page_source: Option<&dyn BudgetPageSource> = match source {
        Some(fake) => Some(fake),
        None => None,
    };
    let request = CustomerRefresh {
        source: page_source,
        lookups,
        default_budget_id,
        gauges,
        end_user_settings: settings,
        context,
    };
    refresh_customers(&request).await;
}

#[tokio::test]
async fn a_missing_database_logs_and_skips_the_customers() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["end_user"]);
    let lookups = FakeLookups::with_budget(None);

    refresh(
        None,
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert!(rig.log.events().is_empty());
    assert!(lookups.default_calls().is_empty());
    assert!(
        logs.text()
            .contains("Prometheus: skipping customer metrics initialization, DB not initialized"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn noop_gauges_skip_before_the_end_user_gate() {
    let (logs, _guard) = capture_logs();
    let source = FakeSource::new();
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));
    let gauges = noop_entity();

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &gauges,
        end_user_tracking_disabled(),
    )
    .await;

    assert!(source.calls().is_empty());
    assert!(lookups.default_calls().is_empty());
    assert!(!logs.text().contains("end_user tracking disabled"));
}

#[tokio::test]
async fn disabled_end_user_tracking_skips_before_the_lookup() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        end_user_tracking_disabled(),
    )
    .await;

    assert!(source.calls().is_empty());
    assert!(lookups.default_calls().is_empty());
    assert!(
        logs.text().contains(
            "Prometheus: skipping customer metrics initialization, end_user tracking disabled"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn prometheus_only_off_skips_before_the_lookup() {
    let (logs, _guard) = capture_logs();
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only_off(),
    )
    .await;

    assert!(source.calls().is_empty());
    assert!(lookups.default_calls().is_empty());
    assert!(
        logs.text().contains(
            "Prometheus: skipping customer metrics initialization, end_user tracking disabled"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn the_default_budget_fills_rows_without_one() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer("end-1", Decimal::from(10), None)],
        total_count: Some(1),
    }));
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert_eq!(source.calls(), vec!["customers:50:1:false"]);
    assert_eq!(lookups.default_calls(), vec!["default-budget"]);
    let remaining = rig.log.for_metric("remaining");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].value, MetricValue::Finite(Decimal::from(90)));
    assert_eq!(
        remaining[0].labels,
        vec![("end_user".to_owned(), "end-1".to_owned())]
    );
    let max = rig.log.for_metric("max");
    assert_eq!(max.len(), 1);
    assert_eq!(max[0].value, MetricValue::Finite(Decimal::from(100)));
}

#[tokio::test]
async fn without_a_default_budget_only_budgeted_rows_are_read() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer(
            "end-2",
            Decimal::from(5),
            Some(snapshot(Some(Decimal::from(20)), None)),
        )],
        total_count: Some(1),
    }));
    let lookups = FakeLookups::with_budget(None);

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert_eq!(source.calls(), vec!["customers:50:1:true"]);
    assert_eq!(lookups.default_calls(), len_one("default-budget"));
    assert_eq!(
        rig.log.for_metric("remaining")[0].value,
        MetricValue::Finite(Decimal::from(15))
    );
}

#[tokio::test]
async fn a_row_without_any_budget_stays_unlimited() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer("end-3", Decimal::from(5), None)],
        total_count: Some(1),
    }));
    let lookups = FakeLookups::with_budget(None);

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert_eq!(
        rig.log.for_metric("remaining")[0].value,
        MetricValue::PositiveInfinity
    );
    assert!(rig.log.for_metric("max").is_empty());
}

#[tokio::test]
async fn a_failed_default_budget_lookup_falls_back_to_budgeted_rows() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer(
            "end-4",
            Decimal::from(10),
            Some(snapshot(Some(Decimal::from(50)), None)),
        )],
        total_count: Some(1),
    }));
    let lookups = FakeLookups::failing();

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert_eq!(source.calls(), vec!["customers:50:1:true"]);
    assert_eq!(lookups.default_calls(), len_one("default-budget"));
    assert_eq!(
        rig.log.for_metric("remaining")[0].value,
        MetricValue::Finite(Decimal::from(40))
    );
}

#[tokio::test]
async fn every_page_reaches_the_customer_gauges_and_reads_the_default_once() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer("end-1", Decimal::from(1), None)],
        total_count: Some(51),
    }));
    source.push_customers(Ok(Page {
        rows: vec![customer("end-2", Decimal::from(2), None)],
        total_count: Some(999),
    }));
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));

    refresh(
        Some(&source),
        &lookups,
        Some("default-budget"),
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert_eq!(
        source.calls(),
        vec!["customers:50:1:false", "customers:50:2:false"]
    );
    assert_eq!(lookups.default_calls(), len_one("default-budget"));
    assert_eq!(rig.log.for_metric("remaining").len(), 2);
}

#[tokio::test]
async fn no_default_budget_id_skips_the_lookup() {
    let rig = budget_rig(&["end_user"]);
    let source = FakeSource::new();
    source.push_customers(Ok(Page {
        rows: vec![customer(
            "end-5",
            Decimal::from(5),
            Some(snapshot(Some(Decimal::from(20)), None)),
        )],
        total_count: Some(1),
    }));
    let lookups = FakeLookups::with_budget(Some(snapshot(Some(Decimal::from(100)), None)));

    refresh(
        Some(&source),
        &lookups,
        None,
        &rig.gauges.customer,
        prometheus_only(),
    )
    .await;

    assert!(lookups.default_calls().is_empty());
    assert_eq!(source.calls(), vec!["customers:50:1:true"]);
    assert_eq!(
        rig.log.for_metric("remaining")[0].value,
        MetricValue::Finite(Decimal::from(15))
    );
}

fn len_one(value: &str) -> Vec<String> {
    vec![value.to_owned()]
}
