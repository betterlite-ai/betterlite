#![cfg(test)]

use std::time::Instant;

use rstest::rstest;

use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_latency::latency_histograms::{
    LatencyObservation, LatencyTimestamps, safe_duration_seconds, set_latency_metrics,
};
use crate::prometheus_latency::test_support::{
    EventLog, Recorded, capture_logs, dec, end_user_settings, four_metrics, label_settings,
    label_value, label_values, latency_histograms, no_limits, observation, observed, recorded,
    removed, supported,
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

#[test]
fn streaming_request_observes_time_to_first_token_once() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(10015, 1)),
            api_call_start: Some(dec(10005, 1)),
            completion_start: Some(dec(100075, 2)),
        },
        Some(true),
        None,
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
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
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(10015, 1)),
            api_call_start: Some(dec(10005, 1)),
            completion_start: Some(dec(100075, 2)),
        },
        stream,
        None,
        None,
    );
    let (captured, _guard) = capture_logs();
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    let recorded_events = recorded(&events);
    assert!(
        !recorded_events
            .iter()
            .any(|event| matches!(event, Recorded::Update { metric: "ttft", .. }))
    );
    assert_eq!(
        recorded_events,
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
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
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
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
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
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
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
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
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
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(1015, 1)),
            api_call_start: Some(dec(1000, 1)),
            completion_start: None,
        },
        None,
        None,
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
        vec![observed("api", &[("model", "gpt-4o")], dec(15, 1))]
    );
}

#[test]
fn api_latency_needs_the_api_call_start() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1010, 1)),
            api_call_start: None,
            completion_start: None,
        },
        None,
        None,
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
        vec![observed("total", &[("model", "gpt-4o")], dec(10, 1))]
    );
}

#[test]
fn total_latency_adds_a_non_negative_queue_time() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1010, 1)),
            api_call_start: None,
            completion_start: None,
        },
        None,
        Some(dec(5, 1)),
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
        vec![
            observed("total", &[("model", "gpt-4o")], dec(15, 1)),
            observed("queue", &[("model", "gpt-4o")], dec(5, 1)),
        ]
    );
}

#[test]
fn negative_queue_time_is_left_out_of_total_and_queue() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1010, 1)),
            api_call_start: None,
            completion_start: None,
        },
        None,
        Some(dec(-5, 1)),
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
        vec![observed("total", &[("model", "gpt-4o")], dec(10, 1))]
    );
}

#[test]
fn zero_queue_time_is_added_and_observed() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1010, 1)),
            api_call_start: None,
            completion_start: None,
        },
        None,
        Some(dec(0, 0)),
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
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
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model", "input_sequence_length", "end_user"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(Some("u"), None);
    let context =
        PrometheusLabelFactoryContext::new(&values, &settings, end_user_settings()).unwrap();
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: Some(&context),
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: Some(dec(1010, 1)),
            api_call_start: Some(dec(1002, 1)),
            completion_start: Some(dec(1003, 1)),
        },
        Some(true),
        Some(dec(5, 1)),
        input_sequence_length,
    );
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    let recorded_events = recorded(&events);
    assert_eq!(recorded_events.len(), 4);
    let lengths: Vec<Option<String>> = recorded_events
        .iter()
        .map(|event| label_value(event, "input_sequence_length"))
        .collect();
    let expected_value = Some(expected.to_owned());
    assert_eq!(
        lengths,
        vec![
            expected_value.clone(),
            expected_value.clone(),
            expected_value.clone(),
            Some("None".to_owned()),
        ]
    );
    let metrics_seen: Vec<&str> = recorded_events
        .iter()
        .map(|event| match event {
            Recorded::Update { metric, .. } => *metric,
            Recorded::Removal { .. } => "removal",
        })
        .collect();
    assert_eq!(metrics_seen, vec!["ttft", "api", "total", "queue"]);
}

#[test]
fn missing_end_time_uses_the_injected_now() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        no_limits(),
    );
    let settings = label_settings();
    let values = label_values(None, None);
    let inputs = LabelInputs {
        values: &values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = LatencyObservation {
        timestamps: LatencyTimestamps {
            start_time: Some(dec(1000, 1)),
            end_time: None,
            api_call_start: Some(dec(1010, 1)),
            completion_start: None,
        },
        now: dec(1050, 1),
        now_monotonic: Instant::now(),
        stream: None,
        queue_time_seconds: None,
        input_sequence_length: None,
    };
    set_latency_metrics(&histograms, &observed_request, &inputs).unwrap();
    assert_eq!(
        recorded(&events),
        vec![
            observed("api", &[("model", "gpt-4o")], dec(40, 1)),
            observed("total", &[("model", "gpt-4o")], dec(50, 1)),
        ]
    );
}

#[test]
fn latency_samples_call_the_end_user_cap_after_observing() {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model", "end_user"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        limits,
    );
    let settings = label_settings();
    let first_values = label_values(Some("e1"), None);
    let first_inputs = LabelInputs {
        values: &first_values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(1010, 1)),
            api_call_start: Some(dec(1000, 1)),
            completion_start: None,
        },
        None,
        None,
        None,
    );
    set_latency_metrics(&histograms, &observed_request, &first_inputs).unwrap();
    let second_values = label_values(Some("e2"), None);
    let second_inputs = LabelInputs {
        values: &second_values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    set_latency_metrics(&histograms, &observed_request, &second_inputs).unwrap();
    assert_eq!(
        recorded(&events),
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
