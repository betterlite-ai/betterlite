#![cfg(test)]

use std::convert::Infallible;

use rstest::rstest;

use crate::prometheus_latency::guardrail_series::{
    GuardrailCollectors, GuardrailObservation, GuardrailStatus, record_guardrail_metrics,
};
use crate::prometheus_latency::test_support::{
    EventLog, FailingSeries, FailurePoint, RecordingMetric, RejectedLabels, capture_logs, dec,
    incremented, observed, recorded,
};

fn guardrail_observation(
    status: GuardrailStatus,
    error_type: Option<&str>,
) -> GuardrailObservation<'_> {
    GuardrailObservation {
        guardrail_name: "pii",
        latency_seconds: dec(25, 2),
        status,
        error_type,
        hook_type: "pre_call",
    }
}

fn infallible_collectors<'collectors>(
    latency: &'collectors RecordingMetric,
    requests: &'collectors RecordingMetric,
    errors: &'collectors RecordingMetric,
) -> GuardrailCollectors<'collectors, Infallible> {
    GuardrailCollectors {
        latency,
        requests,
        errors,
    }
}

#[test]
fn success_records_latency_and_requests_without_an_error() {
    let events = EventLog::default();
    let latency = RecordingMetric::with_log("latency", &events);
    let requests = RecordingMetric::with_log("requests", &events);
    let errors = RecordingMetric::with_log("errors", &events);
    let collectors = infallible_collectors(&latency, &requests, &errors);
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Success, None),
    );
    assert_eq!(
        recorded(&events),
        vec![
            observed(
                "latency",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "success"),
                    ("error_type", "none"),
                    ("hook_type", "pre_call"),
                ],
                dec(25, 2),
            ),
            incremented(
                "requests",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "success"),
                    ("hook_type", "pre_call"),
                ],
            ),
        ]
    );
}

#[test]
fn an_error_with_a_type_increments_all_three_series() {
    let events = EventLog::default();
    let latency = RecordingMetric::with_log("latency", &events);
    let requests = RecordingMetric::with_log("requests", &events);
    let errors = RecordingMetric::with_log("errors", &events);
    let collectors = infallible_collectors(&latency, &requests, &errors);
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Error, Some("timeout")),
    );
    assert_eq!(
        recorded(&events),
        vec![
            observed(
                "latency",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "error"),
                    ("error_type", "timeout"),
                    ("hook_type", "pre_call"),
                ],
                dec(25, 2),
            ),
            incremented(
                "requests",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "error"),
                    ("hook_type", "pre_call"),
                ],
            ),
            incremented(
                "errors",
                &[
                    ("guardrail_name", "pii"),
                    ("error_type", "timeout"),
                    ("hook_type", "pre_call"),
                ],
            ),
        ]
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn an_error_without_a_type_labels_none_and_skips_the_error_counter(
    #[case] error_type: Option<&str>,
) {
    let events = EventLog::default();
    let latency = RecordingMetric::with_log("latency", &events);
    let requests = RecordingMetric::with_log("requests", &events);
    let errors = RecordingMetric::with_log("errors", &events);
    let collectors = infallible_collectors(&latency, &requests, &errors);
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Error, error_type),
    );
    assert_eq!(
        recorded(&events),
        vec![
            observed(
                "latency",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "error"),
                    ("error_type", "none"),
                    ("hook_type", "pre_call"),
                ],
                dec(25, 2),
            ),
            incremented(
                "requests",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "error"),
                    ("hook_type", "pre_call"),
                ],
            ),
        ]
    );
}

#[test]
fn an_intervened_status_with_a_type_skips_the_error_counter() {
    let events = EventLog::default();
    let latency = RecordingMetric::with_log("latency", &events);
    let requests = RecordingMetric::with_log("requests", &events);
    let errors = RecordingMetric::with_log("errors", &events);
    let collectors = infallible_collectors(&latency, &requests, &errors);
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Intervened, Some("pii")),
    );
    assert_eq!(
        recorded(&events),
        vec![
            observed(
                "latency",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "intervened"),
                    ("error_type", "pii"),
                    ("hook_type", "pre_call"),
                ],
                dec(25, 2),
            ),
            incremented(
                "requests",
                &[
                    ("guardrail_name", "pii"),
                    ("status", "intervened"),
                    ("hook_type", "pre_call"),
                ],
            ),
        ]
    );
}

fn failing_collectors<'collectors>(
    latency: &'collectors FailingSeries,
    requests: &'collectors FailingSeries,
    errors: &'collectors FailingSeries,
) -> GuardrailCollectors<'collectors, RejectedLabels> {
    GuardrailCollectors {
        latency,
        requests,
        errors,
    }
}

#[test]
fn a_rejected_latency_sample_is_logged_and_skips_the_later_series() {
    let events = EventLog::default();
    let latency = FailingSeries::with_log("latency", &events, FailurePoint::Observe);
    let requests = FailingSeries::with_log("requests", &events, FailurePoint::NoFailure);
    let errors = FailingSeries::with_log("errors", &events, FailurePoint::NoFailure);
    let collectors = failing_collectors(&latency, &requests, &errors);
    let (captured, _guard) = capture_logs();
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Error, Some("timeout")),
    );
    assert_eq!(recorded(&events), Vec::new());
    assert!(
        captured
            .text()
            .contains("Error recording guardrail metrics")
    );
}

#[test]
fn a_rejected_request_increment_skips_the_error_counter() {
    let events = EventLog::default();
    let latency = FailingSeries::with_log("latency", &events, FailurePoint::NoFailure);
    let requests = FailingSeries::with_log("requests", &events, FailurePoint::Increment);
    let errors = FailingSeries::with_log("errors", &events, FailurePoint::NoFailure);
    let collectors = failing_collectors(&latency, &requests, &errors);
    let (captured, _guard) = capture_logs();
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Error, Some("timeout")),
    );
    assert_eq!(
        recorded(&events),
        vec![observed(
            "latency",
            &[
                ("guardrail_name", "pii"),
                ("status", "error"),
                ("error_type", "timeout"),
                ("hook_type", "pre_call"),
            ],
            dec(25, 2),
        )]
    );
    assert!(
        captured
            .text()
            .contains("Error recording guardrail metrics")
    );
}

#[test]
fn a_rejected_error_increment_is_logged_and_not_returned() {
    let events = EventLog::default();
    let latency = FailingSeries::with_log("latency", &events, FailurePoint::NoFailure);
    let requests = FailingSeries::with_log("requests", &events, FailurePoint::NoFailure);
    let errors = FailingSeries::with_log("errors", &events, FailurePoint::Increment);
    let collectors = failing_collectors(&latency, &requests, &errors);
    let (captured, _guard) = capture_logs();
    record_guardrail_metrics(
        &collectors,
        &guardrail_observation(GuardrailStatus::Error, Some("timeout")),
    );
    let recorded_events = recorded(&events);
    assert_eq!(recorded_events.len(), 2);
    assert!(
        captured
            .text()
            .contains("Error recording guardrail metrics")
    );
}
