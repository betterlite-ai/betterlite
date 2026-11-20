#![cfg(test)]

use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::{BudgetGaugeClock, CustomerGaugeContext, set_customer_budget_metrics};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::inputs::CustomerBudgetGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, finite, hours_value, prometheus_only, removal, rig, set, source,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_customer_budget_metric";
const MAX: &str = "litellm_customer_max_budget_metric";
const HOURS: &str = "litellm_customer_budget_remaining_hours_metric";

fn customer(end_user: &str, spend: &str, max: Option<&str>) -> CustomerBudgetGaugeInput {
    CustomerBudgetGaugeInput {
        end_user_id: end_user.to_owned(),
        spend: decimal(spend),
        max_budget: max.map(decimal),
        budget_reset_at: Some(DateTime::from_timestamp(7200, 0).unwrap()),
    }
}

fn context<'borrow>(
    settings: &'borrow PrometheusLabelSettings,
    tracking: EndUserCostTrackingSettings,
    tracker: &'borrow BoundedPrometheusSeriesTracker,
    limits: &'borrow SeriesLimits,
) -> CustomerGaugeContext<'borrow> {
    CustomerGaugeContext {
        source: source(settings, tracking),
        clock: BudgetGaugeClock {
            reset: DateTime::from_timestamp(0, 0).unwrap(),
            monotonic: Instant::now(),
        },
        tracker,
        limits,
    }
}

#[test]
fn all_three_gauges_are_written_with_the_end_user_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["end_user"]);
    let input = customer("e1", "4", Some("10"));
    set_customer_budget_metrics(
        &gauges,
        &input,
        context(&settings, prometheus_only(), &tracker, &limits),
    )
    .unwrap();
    let labels = [("end_user", "e1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("6")),
            set(MAX, &labels, finite("10")),
            set(HOURS, &labels, hours_value(2.0)),
        ]
    );
}

#[test]
fn a_suppressed_end_user_label_writes_nothing() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["end_user"]);
    let input = customer("e1", "4", Some("10"));
    set_customer_budget_metrics(
        &gauges,
        &input,
        context(
            &settings,
            EndUserCostTrackingSettings::default(),
            &tracker,
            &limits,
        ),
    )
    .unwrap();
    assert!(events(&log).is_empty());
}

#[test]
fn every_gauge_tracks_its_series_and_evicts_the_oldest() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["end_user"]);
    let first = customer("e1", "4", Some("10"));
    let second = customer("e2", "4", Some("10"));
    let shared = context(&settings, prometheus_only(), &tracker, &limits);
    set_customer_budget_metrics(&gauges, &first, shared).unwrap();
    set_customer_budget_metrics(&gauges, &second, shared).unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &[("end_user", "e1")], finite("6")),
            set(MAX, &[("end_user", "e1")], finite("10")),
            set(HOURS, &[("end_user", "e1")], hours_value(2.0)),
            set(REMAINING, &[("end_user", "e2")], finite("6")),
            removal(REMAINING, &[Some("e1")]),
            set(MAX, &[("end_user", "e2")], finite("10")),
            removal(MAX, &[Some("e1")]),
            set(HOURS, &[("end_user", "e2")], hours_value(2.0)),
            removal(HOURS, &[Some("e1")]),
        ]
    );
}

#[test]
fn no_series_limits_leave_the_gauges_untracked() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["end_user"]);
    let first = customer("e1", "4", None);
    set_customer_budget_metrics(
        &gauges,
        &first,
        context(&settings, prometheus_only(), &tracker, &limits),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            REMAINING,
            &[("end_user", "e1")],
            MetricValue::PositiveInfinity,
        )]
    );
}
