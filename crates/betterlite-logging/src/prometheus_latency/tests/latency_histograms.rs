#![cfg(test)]

use rstest::rstest;

use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_latency::latency_histograms::{LatencyTimestamps, safe_duration_seconds};
use crate::prometheus_latency::test_support::{
    Recorded, capture_logs, dec, end_user_cap_events, end_user_settings, label_settings,
    label_value, label_values, no_limits, observation, observed, recorded_events, removed,
};

#[test]
fn safe_duration_needs_both_timestamps_and_keeps_a_negative() {
    assert_eq!(
        safe_duration_seconds(Some(dec(1000, 0)), Some(dec(1001, 0))),
        Some(dec(1, 0))
    );
    assert_eq!(
        safe_duration_seconds(Some(dec(10075, 2)), Some(dec(1005, 1))),
        Some(dec(-25, 2))
    );
    assert_eq!(safe_duration_seconds(None, Some(dec(1, 0))), None);
    assert_eq!(safe_duration_seconds(Some(dec(1, 0)), None), None);
    assert_eq!(safe_duration_seconds(None, None), None);
}

fn input_length_labels(events: &[Recorded]) -> Vec<Option<String>> {
    events
        .iter()
        .map(|event| label_value(event, "input_sequence_length"))
        .collect()
}

fn metric_order(events: &[Recorded]) -> Vec<&str> {
    events
        .iter()
        .map(|event| match event {
            Recorded::Update { metric, .. } => *metric,
            Recorded::Removal { .. } => "removal",
        })
        .collect()
}

#[test]
fn streaming_request_observes_time_to_first_token_once() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: None,
                    end_time: Some(dec(10015, 1)),
                    api_call_start: Some(dec(10005, 1)),
                    completion_start: Some(dec(100_075, 2)),
                },
                Some(true),
                None,
                None,
            ),
        ),
        vec![
            observed("ttft", &[("model", "gpt-4o")], dec(25, 2)),
            observed("api", &[("model", "gpt-4o")], dec(10, 1)),
        ]
    );
}

#[rstest]
#[case::missing_stream(None)]
#[case::false_stream(Some(false))]
fn non_streaming_request_logs_instead_of_observing_time_to_first_token(
    #[case] stream: Option<bool>,
) {
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(10015, 1)),
            api_call_start: Some(dec(10005, 1)),
            completion_start: Some(dec(100_075, 2)),
        },
        stream,
        None,
        None,
    );
    let (captured, _guard) = capture_logs();
    let events = recorded_events(
        &["model"],
        &label_values(None, None),
        None,
        no_limits(),
        &observed_request,
    );
    assert_eq!(
        events,
        vec![observed("api", &[("model", "gpt-4o")], dec(10, 1))]
    );
    assert!(
        captured
            .text()
            .contains("Time to first token metric not emitted")
    );
}

#[test]
fn a_streaming_request_without_a_completion_start_skips_time_to_first_token() {
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(10015, 1)),
            api_call_start: Some(dec(10005, 1)),
            completion_start: None,
        },
        Some(true),
        None,
        None,
    );
    let (captured, _guard) = capture_logs();
    let events = recorded_events(
        &["model"],
        &label_values(None, None),
        None,
        no_limits(),
        &observed_request,
    );
    assert_eq!(
        events,
        vec![observed("api", &[("model", "gpt-4o")], dec(10, 1))]
    );
    assert!(
        captured
            .text()
            .contains("Time to first token metric not emitted")
    );
}

#[test]
fn a_streaming_request_without_an_api_call_start_skips_time_to_first_token_and_api_latency() {
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1015, 1)),
            api_call_start: None,
            completion_start: Some(dec(1003, 1)),
        },
        Some(true),
        None,
        None,
    );
    let (captured, _guard) = capture_logs();
    let events = recorded_events(
        &["model"],
        &label_values(None, None),
        None,
        no_limits(),
        &observed_request,
    );
    assert_eq!(
        events,
        vec![observed("total", &[("model", "gpt-4o")], dec(15, 1))]
    );
    assert!(
        captured
            .text()
            .contains("Time to first token metric not emitted")
    );
}

#[test]
fn api_latency_observes_end_minus_api_call_start() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: None,
                    end_time: Some(dec(1015, 1)),
                    api_call_start: Some(dec(1000, 1)),
                    completion_start: None,
                },
                None,
                None,
                None,
            ),
        ),
        vec![observed("api", &[("model", "gpt-4o")], dec(15, 1))]
    );
}

#[test]
fn api_latency_needs_the_api_call_start() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: Some(dec(1000, 1)),
                    end_time: Some(dec(1010, 1)),
                    api_call_start: None,
                    completion_start: None,
                },
                None,
                None,
                None,
            ),
        ),
        vec![observed("total", &[("model", "gpt-4o")], dec(10, 1))]
    );
}

#[test]
fn total_latency_adds_a_non_negative_queue_time() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: Some(dec(1000, 1)),
                    end_time: Some(dec(1010, 1)),
                    api_call_start: None,
                    completion_start: None,
                },
                None,
                Some(dec(5, 1)),
                None,
            ),
        ),
        vec![
            observed("total", &[("model", "gpt-4o")], dec(15, 1)),
            observed("queue", &[("model", "gpt-4o")], dec(5, 1)),
        ]
    );
}

#[test]
fn negative_queue_time_is_left_out_of_total_and_queue() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: Some(dec(1000, 1)),
                    end_time: Some(dec(1010, 1)),
                    api_call_start: None,
                    completion_start: None,
                },
                None,
                Some(dec(-5, 1)),
                None,
            ),
        ),
        vec![observed("total", &[("model", "gpt-4o")], dec(10, 1))]
    );
}

#[test]
fn zero_queue_time_is_added_and_observed() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: Some(dec(1000, 1)),
                    end_time: Some(dec(1010, 1)),
                    api_call_start: None,
                    completion_start: None,
                },
                None,
                Some(dec(0, 0)),
                None,
            ),
        ),
        vec![
            observed("total", &[("model", "gpt-4o")], dec(10, 1)),
            observed("queue", &[("model", "gpt-4o")], dec(0, 0)),
        ]
    );
}

#[rstest]
#[case::present(Some("0-1k"), "0-1k")]
#[case::absent(None, "None")]
fn input_sequence_length_reaches_the_first_three_histograms_only(
    #[case] input_sequence_length: Option<&str>,
    #[case] expected: &str,
) {
    let values = label_values(Some("u"), None);
    let settings = label_settings();
    let context =
        PrometheusLabelFactoryContext::new(&values, &settings, end_user_settings()).unwrap();
    let events = recorded_events(
        &["model", "input_sequence_length", "end_user"],
        &values,
        Some(&context),
        no_limits(),
        &observation(
            LatencyTimestamps {
                start_time: Some(dec(1000, 1)),
                end_time: Some(dec(1010, 1)),
                api_call_start: Some(dec(1002, 1)),
                completion_start: Some(dec(1003, 1)),
            },
            Some(true),
            Some(dec(5, 1)),
            input_sequence_length,
        ),
    );
    assert_eq!(events.len(), 4);
    let expected_value = Some(expected.to_owned());
    assert_eq!(
        input_length_labels(&events),
        vec![
            expected_value.clone(),
            expected_value.clone(),
            expected_value,
            Some("None".to_owned()),
        ]
    );
    assert_eq!(metric_order(&events), vec!["ttft", "api", "total", "queue"]);
}

#[test]
fn missing_end_time_uses_the_injected_now() {
    assert_eq!(
        recorded_events(
            &["model"],
            &label_values(None, None),
            None,
            no_limits(),
            &observation(
                LatencyTimestamps {
                    start_time: Some(dec(1000, 1)),
                    end_time: None,
                    api_call_start: Some(dec(1010, 1)),
                    completion_start: None,
                },
                None,
                None,
                None,
            ),
        ),
        vec![
            observed("api", &[("model", "gpt-4o")], dec(18990, 1)),
            observed("total", &[("model", "gpt-4o")], dec(19000, 1)),
        ]
    );
}

#[test]
fn latency_samples_call_the_end_user_cap_after_observing() {
    let first = label_values(Some("e1"), None);
    let second = label_values(Some("e2"), None);
    assert_eq!(
        end_user_cap_events([&first, &second]),
        vec![
            observed(
                "api",
                &[("end_user", "e1"), ("model", "gpt-4o")],
                dec(10, 1)
            ),
            observed(
                "api",
                &[("end_user", "e2"), ("model", "gpt-4o")],
                dec(10, 1)
            ),
            removed("api", &[Some("gpt-4o"), Some("e1")]),
        ]
    );
}
