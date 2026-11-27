#![cfg(test)]

use super::{CustomerBudgetRequest, set_customer_budget_metrics_after_api_request};
use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_lookups::records::CachedEndUser;
use crate::prometheus::budget_lookups::test_support::{
    Answer, CUSTOMER_GAUGES, LookupCall, RecordingLookups, all_samples, capture_logs,
    customer_context, customer_tracking, decimal, default_tracking, entity_gauges, infinity_row,
    row, time,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 1] = ["end_user"];

fn budget_table(max_budget: Option<&str>, reset_at: Option<&str>) -> BudgetTableSnapshot {
    BudgetTableSnapshot {
        max_budget: max_budget.map(decimal),
        budget_reset_at: reset_at.map(time),
    }
}

#[tokio::test]
async fn a_cache_hit_adds_the_cost_and_emits_the_cached_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups {
        customer: Answer::Found(CachedEndUser {
            spend: decimal("1.5"),
            litellm_budget_table: Some(budget_table(Some("10"), Some("2026-01-06T00:00:00Z"))),
        }),
        ..RecordingLookups::default()
    };
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
    let labels = [("end_user", "end-1")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(CUSTOMER_GAUGES[0], &labels, "8.25"),
            row(CUSTOMER_GAUGES[1], &labels, "10"),
        ]
    );
    assert_eq!(
        lookups.calls(),
        vec![LookupCall::CachedCustomer {
            cache_key: "end_user_id:end-1".to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_cache_hit_without_a_table_emits_only_the_remaining_gauge() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups {
        customer: Answer::Found(CachedEndUser {
            spend: decimal("2"),
            litellm_budget_table: None,
        }),
        ..RecordingLookups::default()
    };
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
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(CUSTOMER_GAUGES[0], &[("end_user", "end-1")])]
    );
}

#[tokio::test]
async fn a_cache_miss_emits_nothing() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default();
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
    assert!(all_samples(&gauges).is_empty());
    assert_eq!(lookups.calls().len(), 1);
}

#[tokio::test]
async fn a_lookup_error_emits_nothing_and_logs_it() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups {
        customer: Answer::Failed("the end-user cache read failed"),
        ..RecordingLookups::default()
    };
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
    assert!(all_samples(&gauges).is_empty());
    assert!(
        logs.text()
            .contains("Error getting customer info: the end-user cache read failed")
    );
}

#[tokio::test]
async fn no_end_user_id_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(CUSTOMER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let customer_request = CustomerBudgetRequest {
        end_user_id: None,
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
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn noop_gauges_skip_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(CUSTOMER_GAUGES, &LABELS, true);
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
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

// a hit without a limit still writes the remaining series as infinity, never the max or hours one
#[tokio::test]
async fn a_cache_hit_with_a_limit_gauge_but_no_max_writes_no_max() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups {
        customer: Answer::Found(CachedEndUser {
            spend: decimal("2"),
            litellm_budget_table: Some(budget_table(None, Some("2026-01-06T00:00:00Z"))),
        }),
        ..RecordingLookups::default()
    };
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
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(CUSTOMER_GAUGES[0], &[("end_user", "end-1")])]
    );
}

// the settings argument is the gate: a disabled pair skips even though the gauges and the id are ready
#[tokio::test]
async fn a_disabled_config_gate_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(CUSTOMER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let customer_request = CustomerBudgetRequest {
        end_user_id: Some("end-1"),
        response_cost: decimal("0.25"),
    };
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        customer_context(&settings, &tracker, &limits, default_tracking(), now),
        default_tracking(),
        &customer_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}
