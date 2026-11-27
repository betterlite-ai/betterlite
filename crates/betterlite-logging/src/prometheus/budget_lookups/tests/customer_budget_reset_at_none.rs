#![cfg(test)]

use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest, set_customer_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::records::CachedEndUser;
use crate::prometheus::budget_lookups::test_support::{
    Answer, CUSTOMER_GAUGES, RecordingLookups, SampleRow, all_samples, customer_context,
    customer_tracking, decimal, entity_gauges, infinity_row, row, time,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 1] = ["end_user"];

async fn rows_for(cached: CachedEndUser) -> Vec<SampleRow> {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default().with_customer(Answer::Found(cached));
    let gauges = entity_gauges(CUSTOMER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let customer_request = CustomerBudgetRequest {
        end_user_id: Some("end-1"),
        response_cost: decimal("0.25"),
    };
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &customer_request,
    )
    .await
    .unwrap();
    all_samples(&gauges)
}

#[tokio::test]
async fn a_cached_reset_time_never_reaches_the_hours_gauge() {
    let cached = CachedEndUser {
        spend: decimal("1.5"),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: Some(decimal("10")),
            budget_reset_at: Some(time("2026-01-06T00:00:00Z")),
        }),
    };
    let labels = [("end_user", "end-1")];
    assert_eq!(
        rows_for(cached).await,
        vec![
            row(CUSTOMER_GAUGES[0], &labels, "8.25"),
            row(CUSTOMER_GAUGES[1], &labels, "10"),
        ]
    );
}

#[tokio::test]
async fn a_cached_reset_time_without_a_max_leaves_the_remaining_gauge_alone() {
    let cached = CachedEndUser {
        spend: decimal("2"),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: None,
            budget_reset_at: Some(time("2026-01-06T00:00:00Z")),
        }),
    };
    assert_eq!(
        rows_for(cached).await,
        vec![infinity_row(CUSTOMER_GAUGES[0], &[("end_user", "end-1")])]
    );
}

#[tokio::test]
async fn a_past_cached_reset_time_is_dropped_too() {
    let cached = CachedEndUser {
        spend: decimal("1.5"),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: Some(decimal("10")),
            budget_reset_at: Some(time("2025-12-01T00:00:00Z")),
        }),
    };
    let labels = [("end_user", "end-1")];
    assert_eq!(
        rows_for(cached).await,
        vec![
            row(CUSTOMER_GAUGES[0], &labels, "8.25"),
            row(CUSTOMER_GAUGES[1], &labels, "10"),
        ]
    );
}
