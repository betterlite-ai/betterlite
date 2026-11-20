#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use rstest::rstest;

use super::{SuccessEvent, log_success_event, success_event_metrics};
use crate::alerting::deployment_reports::tests::support::{
    FakeCache, FixedClock, Increment, fixed_now,
};

fn event(id: &str, start: f64, end: f64, tokens: Option<i64>) -> SuccessEvent<'_> {
    SuccessEvent {
        deployment_id: id,
        start_time: start,
        end_time: end,
        completion_tokens: tokens,
    }
}

#[rstest]
#[case::three_tokens(Some(3), 0.5)]
#[case::zero_tokens(Some(0), 1.5)]
#[case::negative_tokens(Some(-2), 1.5)]
#[case::no_tokens(None, 1.5)]
fn the_latency_is_the_elapsed_time_per_output_token(
    #[case] tokens: Option<i64>,
    #[case] expected: f64,
) {
    let record = success_event_metrics(
        &FixedClock(fixed_now()),
        &[AlertType::DailyReports],
        &event("d1", 100.0, 101.5, tokens),
    )
    .expect("a daily report record");

    assert_eq!(
        record.latency_per_output_token.map(f64::to_bits),
        Some(expected.to_bits())
    );
}

#[test]
fn the_success_event_records_the_deployment_and_the_clock_instant() {
    let record = success_event_metrics(
        &FixedClock(fixed_now()),
        &[AlertType::DailyReports],
        &event("dep-1", 100.0, 102.0, None),
    )
    .expect("a daily report record");

    assert_eq!(record.id, "dep-1");
    assert!(!record.failed_request);
    assert_eq!(record.updated_at, fixed_now());
    assert_eq!(
        record.latency_per_output_token.map(f64::to_bits),
        Some(2.0_f64.to_bits())
    );
}

#[test]
fn a_disabled_daily_report_type_builds_no_record() {
    let record = success_event_metrics(
        &FixedClock(fixed_now()),
        &[AlertType::FallbackReports],
        &event("d1", 100.0, 101.5, Some(3)),
    );

    assert!(record.is_none());
}

#[tokio::test]
async fn the_success_event_increments_the_latency_metric() {
    let cache = FakeCache::default();

    let updated = log_success_event(
        &cache,
        &FixedClock(fixed_now()),
        &[AlertType::DailyReports],
        &event("d1", 100.0, 101.5, Some(3)),
    )
    .await;

    assert_eq!(updated, 1);
    assert_eq!(
        cache.increments.entries(),
        vec![Increment {
            key: "d1:latency_daily_metrics".to_owned(),
            amount: 0.5,
        }]
    );
}

#[tokio::test]
async fn the_logger_updates_nothing_without_the_daily_report_type() {
    let cache = FakeCache::default();

    let updated = log_success_event(
        &cache,
        &FixedClock(fixed_now()),
        &[AlertType::FallbackReports],
        &event("d1", 100.0, 101.5, Some(3)),
    )
    .await;

    assert_eq!(updated, 0);
    assert!(cache.increments.entries().is_empty());
}
