use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;

use super::support::{
    DEFAULT_SUCCESS, Failure, GROUP, Success, TestClock, latency, minutes, record_elapsed,
    record_failure, record_success, samples, service, success_plan, ttft, usage,
};
use crate::latency_observations::LatencyObservations;
use crate::latency_observations::error::LatencyError;
use crate::latency_observations::event::{Channel, RequestChannels};
use crate::latency_observations::samples::MinuteUsage;
use crate::latency_observations::settings::Settings;
use crate::latency_sample_math::{average, normalized, percentile};
use crate::routing_value_store::Clock;

type BoundaryCase = (&'static str, fn());

const BOUNDARIES: &[BoundaryCase] = &[
    (
        "a_full_window_keeps_every_sample",
        a_full_window_keeps_every_sample,
    ),
    (
        "the_sample_after_a_full_window_drops_the_oldest",
        the_sample_after_a_full_window_drops_the_oldest,
    ),
    (
        "a_zero_or_negative_window_records_nothing",
        a_zero_or_negative_window_records_nothing,
    ),
    (
        "the_ttft_window_drops_its_oldest_at_the_limit",
        the_ttft_window_drops_its_oldest_at_the_limit,
    ),
    (
        "a_sample_at_the_exact_expiry_keeps_the_running_window",
        a_sample_at_the_exact_expiry_keeps_the_running_window,
    ),
    (
        "a_sample_one_millisecond_past_the_expiry_starts_over",
        a_sample_one_millisecond_past_the_expiry_starts_over,
    ),
    (
        "a_sample_just_before_the_expiry_keeps_the_running_deadline",
        a_sample_just_before_the_expiry_keeps_the_running_deadline,
    ),
    (
        "a_failure_just_past_the_expiry_starts_a_new_entry",
        a_failure_just_past_the_expiry_starts_a_new_entry,
    ),
    (
        "the_penalty_window_is_bounded",
        the_penalty_window_is_bounded,
    ),
    (
        "a_percentile_at_the_exact_rank_takes_that_sample",
        a_percentile_at_the_exact_rank_takes_that_sample,
    ),
    (
        "a_percentile_at_one_takes_the_largest_sample",
        a_percentile_at_one_takes_the_largest_sample,
    ),
    (
        "an_empty_window_averages_to_zero_and_has_no_percentile",
        an_empty_window_has_no_score,
    ),
    (
        "the_average_divides_the_sum_by_the_sample_count",
        the_average_divides_the_sum_by_the_sample_count,
    ),
    (
        "a_non_positive_completion_count_has_no_normalized_time",
        a_non_positive_completion_count_has_no_normalized_time,
    ),
    (
        "a_minute_total_past_the_integer_limit_is_an_error",
        a_minute_total_past_the_integer_limit_is_an_error,
    ),
    (
        "a_model_group_past_the_key_limit_is_an_error",
        a_model_group_past_the_key_limit_is_an_error,
    ),
    (
        "a_cached_value_the_callback_did_not_write_is_an_error",
        a_cached_value_the_callback_did_not_write_is_an_error,
    ),
    ("a_negative_clock_is_an_error", a_negative_clock_is_an_error),
];

pub(crate) fn run(name: &str) {
    match BOUNDARIES
        .iter()
        .find(|(case_name, _run)| *case_name == name)
    {
        Some((_case, run)) => run(),
        None => panic!("unknown boundary case {name}"),
    }
}

fn a_full_window_keeps_every_sample() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_elapsed(
        &observations,
        &[
            1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 9_000, 10_000,
        ],
    );
    assert_eq!(
        latency(&observations),
        vec![
            1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64, 5.0_f64, 6.0_f64, 7.0_f64, 8.0_f64, 9.0_f64,
            10.0_f64
        ]
    );
}

fn the_sample_after_a_full_window_drops_the_oldest() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_elapsed(
        &observations,
        &[
            1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 9_000, 10_000, 11_000,
        ],
    );
    assert_eq!(
        latency(&observations),
        vec![
            2.0_f64, 3.0_f64, 4.0_f64, 5.0_f64, 6.0_f64, 7.0_f64, 8.0_f64, 9.0_f64, 10.0_f64,
            11.0_f64
        ]
    );
}

fn a_zero_or_negative_window_records_nothing() {
    let clock = TestClock::at(0);
    let zero = service(settings(3_600.0_f64, 0), &clock);
    record_elapsed(&zero, &[1_000, 2_000, 3_000]);
    record_failure(&zero, &Failure::default());
    assert!(samples(&zero, GROUP).deployments.is_empty());
    let negative = service(settings(3_600.0_f64, -1), &clock);
    record_elapsed(&negative, &[1_000, 2_000, 3_000]);
    record_failure(&negative, &Failure::default());
    assert!(samples(&negative, GROUP).deployments.is_empty());
}

fn the_ttft_window_drops_its_oldest_at_the_limit() {
    let clock = TestClock::at(0);
    let observations = service(settings(3_600.0_f64, 2), &clock);
    for first_token in [1_000, 2_000, 3_000] {
        record_success(
            &observations,
            &Success {
                stream: true,
                elapsed_millis: 4_000,
                completion_start_millis: Some(first_token),
                ..DEFAULT_SUCCESS
            },
        );
    }
    assert_eq!(ttft(&observations), vec![2.0_f64, 3.0_f64]);
    assert_eq!(latency(&observations), vec![4.0_f64, 4.0_f64]);
}

fn a_sample_at_the_exact_expiry_keeps_the_running_window() {
    let clock = TestClock::at(0);
    let observations = service(settings(60.0_f64, 10), &clock);
    record_success(&observations, &Success::default());
    clock.set(60_000);
    record_success(&observations, &Success::default());
    assert_eq!(latency(&observations), vec![1.0_f64, 1.0_f64]);
}

fn a_sample_one_millisecond_past_the_expiry_starts_over() {
    let clock = TestClock::at(0);
    let observations = service(settings(60.0_f64, 10), &clock);
    record_success(&observations, &success_plan(1_000, usage(1, 4)));
    clock.set(60_001);
    record_success(&observations, &success_plan(2_000, usage(1, 9)));
    assert_eq!(latency(&observations), vec![2.0_f64]);
    assert_eq!(
        minutes(&observations),
        vec![(
            "1970-01-01-00-01".to_owned(),
            MinuteUsage { tpm: 9, rpm: 1 }
        )]
    );
}

fn a_sample_just_before_the_expiry_keeps_the_running_deadline() {
    let clock = TestClock::at(0);
    let observations = service(settings(60.0_f64, 10), &clock);
    record_success(&observations, &Success::default());
    clock.set(59_999);
    record_success(&observations, &Success::default());
    clock.set(60_000);
    assert_eq!(latency(&observations), vec![1.0_f64, 1.0_f64]);
    clock.set(60_001);
    assert!(latency(&observations).is_empty());
}

fn a_failure_just_past_the_expiry_starts_a_new_entry() {
    let clock = TestClock::at(0);
    let observations = service(settings(60.0_f64, 10), &clock);
    record_success(&observations, &Success::default());
    clock.set(60_001);
    record_failure(&observations, &Failure::default());
    assert_eq!(latency(&observations), vec![1000.0_f64]);
}

fn the_penalty_window_is_bounded() {
    let clock = TestClock::at(0);
    let observations = service(settings(3_600.0_f64, 2), &clock);
    record_failure(&observations, &Failure::default());
    record_failure(&observations, &Failure::default());
    record_failure(&observations, &Failure::default());
    assert_eq!(latency(&observations), vec![1000.0_f64, 1000.0_f64]);
}

fn a_percentile_at_the_exact_rank_takes_that_sample() {
    let samples = [1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64];
    assert_eq!(percentile(&samples, 0.25_f64), Some(1.0_f64));
    assert_eq!(percentile(&samples, 0.5_f64), Some(2.0_f64));
    assert_eq!(percentile(&samples, 0.250_000_1_f64), Some(2.0_f64));
}

fn a_percentile_at_one_takes_the_largest_sample() {
    let samples = [3.0_f64, 1.0_f64, 2.0_f64];
    assert_eq!(percentile(&samples, 0.5_f64), Some(2.0_f64));
    assert_eq!(percentile(&samples, 0.75_f64), Some(3.0_f64));
    assert_eq!(percentile(&samples, 1.0_f64), Some(3.0_f64));
}

fn an_empty_window_has_no_score() {
    assert_eq!(average(&[]).to_bits(), 0.0_f64.to_bits());
    assert_eq!(percentile(&[], 0.5_f64), None);
}

fn the_average_divides_the_sum_by_the_sample_count() {
    assert_eq!(
        average(&[1.0_f64, 2.0_f64, 3.0_f64]).to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(average(&[0.5_f64]).to_bits(), 0.5_f64.to_bits());
}

fn a_non_positive_completion_count_has_no_normalized_time() {
    assert_eq!(normalized(2.0_f64, 4), Some(0.5_f64));
    assert_eq!(normalized(2.0_f64, 0), None);
    assert_eq!(normalized(2.0_f64, -3), None);
}

fn a_minute_total_past_the_integer_limit_is_an_error() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(&observations, &success_plan(1_000, usage(1, i64::MAX)));
    let second = success_plan(1_000, usage(1, 1));
    let error = observations
        .record_success(&second.event())
        .expect_err("the minute total overflows");
    assert!(matches!(error, LatencyError::Overflow));
    assert_eq!(
        minutes(&observations),
        vec![(
            "1970-01-01-00-00".to_owned(),
            MinuteUsage {
                tpm: i64::MAX,
                rpm: 1
            }
        )]
    );
}

fn a_model_group_past_the_key_limit_is_an_error() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let group = "g".repeat(1_100);
    let plan = Success {
        channels: RequestChannels {
            litellm_metadata: Channel::Absent,
            metadata: Channel::Present(Some(group.as_str())),
        },
        ..DEFAULT_SUCCESS
    };
    let outcome = observations.record_success(&plan.event());
    assert!(matches!(outcome, Err(LatencyError::Key { .. })));
}

fn a_cached_value_the_callback_did_not_write_is_an_error() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    observations
        .cache
        .lock()
        .expect("the test cache lock")
        .set("gpt_map", CacheValue::Null, None)
        .expect("the test cache stores");
    let error = observations
        .samples(GROUP)
        .expect_err("the stored value is not sample text");
    assert!(matches!(error, LatencyError::UnexpectedValue));
}

fn a_negative_clock_is_an_error() {
    let clock: Clock = Arc::new(|| -1.0_f64);
    let observations =
        LatencyObservations::new(Settings::default(), clock).expect("the test cache starts");
    let error = observations
        .record_success(&Success::default().event())
        .expect_err("the clock is not a valid instant");
    assert!(matches!(error, LatencyError::Clock { .. }));
}

fn settings(ttl_seconds: f64, max_latency_list_size: i64) -> Settings {
    Settings {
        ttl_seconds,
        lowest_latency_buffer: 0.0_f64,
        max_latency_list_size,
        ttft_percentile: None,
    }
}
