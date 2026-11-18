use std::sync::Arc;

use super::support::{
    Failure, GROUP, Success, TestClock, latency, minutes, record_elapsed, record_failure,
    record_success, samples, service, ttft, usage,
};
use crate::latency_observations::LatencyObservations;
use crate::latency_observations::error::LatencyError;
use crate::latency_observations::event::{Channel, RequestChannels};
use crate::latency_observations::samples::MinuteUsage;
use crate::latency_observations::settings::Settings;
use crate::latency_sample_math::{average, normalized, percentile};
use crate::routing_value_store::Clock;

pub(crate) fn run(name: &str) {
    match name {
        "a_full_window_keeps_every_sample" => a_full_window_keeps_every_sample(),
        "the_sample_after_a_full_window_drops_the_oldest" => {
            the_sample_after_a_full_window_drops_the_oldest();
        }
        "a_zero_or_negative_window_records_nothing" => {
            a_zero_or_negative_window_records_nothing();
        }
        "the_ttft_window_drops_its_oldest_at_the_limit" => {
            the_ttft_window_drops_its_oldest_at_the_limit();
        }
        "a_sample_at_the_exact_expiry_keeps_the_running_window" => {
            a_sample_at_the_exact_expiry_keeps_the_running_window();
        }
        "a_sample_one_millisecond_past_the_expiry_starts_over" => {
            a_sample_one_millisecond_past_the_expiry_starts_over();
        }
        "a_sample_just_before_the_expiry_keeps_the_running_deadline" => {
            a_sample_just_before_the_expiry_keeps_the_running_deadline();
        }
        "a_failure_just_past_the_expiry_starts_a_new_entry" => {
            a_failure_just_past_the_expiry_starts_a_new_entry();
        }
        "the_penalty_window_is_bounded" => the_penalty_window_is_bounded(),
        "a_percentile_at_the_exact_rank_takes_that_sample" => {
            a_percentile_at_the_exact_rank_takes_that_sample();
        }
        "a_percentile_at_one_takes_the_largest_sample" => {
            a_percentile_at_one_takes_the_largest_sample();
        }
        "an_empty_window_averages_to_zero_and_has_no_percentile" => {
            an_empty_window_has_no_score();
        }
        "a_non_positive_completion_count_has_no_normalized_time" => {
            a_non_positive_completion_count_has_no_normalized_time();
        }
        "a_minute_total_past_the_integer_limit_is_an_error" => {
            a_minute_total_past_the_integer_limit_is_an_error();
        }
        "a_model_group_past_the_key_limit_is_an_error" => {
            a_model_group_past_the_key_limit_is_an_error();
        }
        "a_negative_clock_is_an_error" => a_negative_clock_is_an_error(),
        other => panic!("unknown boundary case {other}"),
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
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
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
        vec![2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]
    );
}

fn a_zero_or_negative_window_records_nothing() {
    let clock = TestClock::at(0);
    let mut settings = Settings::default();
    settings.max_latency_list_size = 0;
    let zero = service(settings, &clock);
    record_elapsed(&zero, &[1_000, 2_000, 3_000]);
    record_failure(&zero, &Failure::default());
    assert!(samples(&zero, GROUP).deployments.is_empty());
    settings.max_latency_list_size = -1;
    let negative = service(settings, &clock);
    record_elapsed(&negative, &[1_000, 2_000, 3_000]);
    record_failure(&negative, &Failure::default());
    assert!(samples(&negative, GROUP).deployments.is_empty());
}

fn the_ttft_window_drops_its_oldest_at_the_limit() {
    let clock = TestClock::at(0);
    let mut settings = Settings::default();
    settings.max_latency_list_size = 2;
    let observations = service(settings, &clock);
    for first_token in [1_000, 2_000, 3_000] {
        let mut plan = Success::default();
        plan.stream = true;
        plan.elapsed_millis = 4_000;
        plan.completion_start_millis = Some(first_token);
        record_success(&observations, &plan);
    }
    assert_eq!(ttft(&observations), vec![2.0, 3.0]);
    assert_eq!(latency(&observations), vec![4.0, 4.0]);
}

fn a_sample_at_the_exact_expiry_keeps_the_running_window() {
    let clock = TestClock::at(0);
    let observations = service(ttl_settings(60.0), &clock);
    record_success(&observations, &Success::default());
    clock.set(60_000);
    record_success(&observations, &Success::default());
    assert_eq!(latency(&observations), vec![1.0, 1.0]);
}

fn a_sample_one_millisecond_past_the_expiry_starts_over() {
    let clock = TestClock::at(0);
    let observations = service(ttl_settings(60.0), &clock);
    let mut first = Success::default();
    first.usage = usage(1, 4);
    record_success(&observations, &first);
    clock.set(60_001);
    let mut second = Success::default();
    second.elapsed_millis = 2_000;
    second.usage = usage(1, 9);
    record_success(&observations, &second);
    assert_eq!(latency(&observations), vec![2.0]);
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
    let observations = service(ttl_settings(60.0), &clock);
    record_success(&observations, &Success::default());
    clock.set(59_999);
    record_success(&observations, &Success::default());
    clock.set(60_000);
    assert_eq!(latency(&observations), vec![1.0, 1.0]);
    clock.set(60_001);
    assert!(latency(&observations).is_empty());
}

fn a_failure_just_past_the_expiry_starts_a_new_entry() {
    let clock = TestClock::at(0);
    let observations = service(ttl_settings(60.0), &clock);
    record_success(&observations, &Success::default());
    clock.set(60_001);
    record_failure(&observations, &Failure::default());
    assert_eq!(latency(&observations), vec![1000.0]);
}

fn the_penalty_window_is_bounded() {
    let clock = TestClock::at(0);
    let mut settings = Settings::default();
    settings.max_latency_list_size = 2;
    let observations = service(settings, &clock);
    record_failure(&observations, &Failure::default());
    record_failure(&observations, &Failure::default());
    record_failure(&observations, &Failure::default());
    assert_eq!(latency(&observations), vec![1000.0, 1000.0]);
}

fn a_percentile_at_the_exact_rank_takes_that_sample() {
    let samples = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(percentile(&samples, 0.25), Some(1.0));
    assert_eq!(percentile(&samples, 0.5), Some(2.0));
    assert_eq!(percentile(&samples, 0.250_000_1), Some(2.0));
}

fn a_percentile_at_one_takes_the_largest_sample() {
    let samples = [3.0, 1.0, 2.0];
    assert_eq!(percentile(&samples, 0.5), Some(2.0));
    assert_eq!(percentile(&samples, 0.75), Some(3.0));
    assert_eq!(percentile(&samples, 1.0), Some(3.0));
}

fn an_empty_window_has_no_score() {
    assert_eq!(average(&[]), 0.0);
    assert_eq!(percentile(&[], 0.5), None);
}

fn a_non_positive_completion_count_has_no_normalized_time() {
    assert_eq!(normalized(2.0, 4), Some(0.5));
    assert_eq!(normalized(2.0, 0), None);
    assert_eq!(normalized(2.0, -3), None);
}

fn a_minute_total_past_the_integer_limit_is_an_error() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut first = Success::default();
    first.usage = usage(1, i64::MAX);
    record_success(&observations, &first);
    let mut second = Success::default();
    second.usage = usage(1, 1);
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
    let mut plan = Success::default();
    plan.channels = RequestChannels {
        litellm_metadata: Channel::Absent,
        metadata: Channel::Present(Some(group.as_str())),
    };
    let error = observations
        .record_success(&plan.event())
        .expect_err("the sample key is too long");
    assert!(matches!(error, LatencyError::Key { .. }));
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

fn ttl_settings(ttl_seconds: f64) -> Settings {
    let mut settings = Settings::default();
    settings.ttl_seconds = ttl_seconds;
    settings
}
