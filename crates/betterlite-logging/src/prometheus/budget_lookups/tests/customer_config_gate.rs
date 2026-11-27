#![cfg(test)]

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest, set_customer_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::records::CachedEndUser;
use crate::prometheus::budget_lookups::test_support::{
    Answer, CUSTOMER_GAUGES, RecordingLookups, SampleRow, all_samples, customer_context,
    customer_tracking, decimal, entity_gauges, row, time,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 1] = ["end_user"];

fn cached_customer() -> Answer<CachedEndUser> {
    Answer::Found(CachedEndUser {
        spend: decimal("1.5"),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: Some(decimal("10")),
            budget_reset_at: None,
        }),
    })
}

async fn emit_with(
    tracking: EndUserCostTrackingSettings,
    noop: bool,
) -> (RecordingLookups, Vec<SampleRow>) {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default().with_customer(cached_customer());
    let gauges = entity_gauges(CUSTOMER_GAUGES, &LABELS, noop);
    let now = time("2026-01-01T00:00:00Z");
    let customer_request = CustomerBudgetRequest {
        end_user_id: Some("end-1"),
        response_cost: decimal("0.25"),
    };
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        customer_context(&settings, &tracker, &limits, tracking, now),
        tracking,
        &customer_request,
    )
    .await
    .unwrap();
    let rows = all_samples(&gauges);
    (lookups, rows)
}

#[tokio::test]
async fn the_default_settings_disable_the_lookup() {
    let (lookups, rows) = emit_with(
        EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: None,
        },
        false,
    )
    .await;
    assert!(rows.is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_prometheus_only_setting_enables_the_lookup() {
    let (lookups, rows) = emit_with(customer_tracking(), false).await;
    let labels = [("end_user", "end-1")];
    assert_eq!(
        rows,
        vec![
            row(CUSTOMER_GAUGES[0], &labels, "8.25"),
            row(CUSTOMER_GAUGES[1], &labels, "10"),
        ]
    );
    assert_eq!(lookups.calls().len(), 1);
}

#[tokio::test]
async fn the_disable_flag_wins_over_prometheus_only() {
    let (lookups, rows) = emit_with(
        EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: Some(true),
            enable_prometheus_only: Some(true),
        },
        false,
    )
    .await;
    assert!(rows.is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_false_prometheus_only_setting_disables_the_lookup() {
    let (lookups, rows) = emit_with(
        EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: Some(false),
        },
        false,
    )
    .await;
    assert!(rows.is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_disable_flag_that_is_false_alone_leaves_the_lookup_off() {
    let (lookups, rows) = emit_with(
        EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: Some(false),
            enable_prometheus_only: None,
        },
        false,
    )
    .await;
    assert!(rows.is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_false_disable_flag_with_prometheus_only_enables_the_lookup() {
    let (lookups, rows) = emit_with(
        EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: Some(false),
            enable_prometheus_only: Some(true),
        },
        false,
    )
    .await;
    assert!(!rows.is_empty());
    assert_eq!(lookups.calls().len(), 1);
}

#[tokio::test]
async fn noop_gauges_disable_the_lookup() {
    let (lookups, rows) = emit_with(customer_tracking(), true).await;
    assert!(rows.is_empty());
    assert!(lookups.calls().is_empty());
}
