#![cfg(test)]
use rstest::rstest;

use super::normalize_otel_endpoint;
use crate::otel::config::Signal;

#[rstest]
#[case::absent(None, Signal::Traces, None)]
#[case::empty(Some(""), Signal::Traces, Some(""))]
#[case::empty_metrics(Some(""), Signal::Metrics, Some(""))]
#[case::base(
    Some("http://host:4318"),
    Signal::Traces,
    Some("http://host:4318/v1/traces")
)]
#[case::trailing_slashes(
    Some("http://host:4318///"),
    Signal::Traces,
    Some("http://host:4318/v1/traces")
)]
#[case::only_slashes(Some("///"), Signal::Logs, Some("/v1/logs"))]
#[case::correct_path(
    Some("http://host/v1/traces"),
    Signal::Traces,
    Some("http://host/v1/traces")
)]
#[case::correct_path_trailing(
    Some("http://host/v1/traces/"),
    Signal::Traces,
    Some("http://host/v1/traces")
)]
#[case::from_logs(
    Some("http://host/v1/logs"),
    Signal::Traces,
    Some("http://host/v1/traces")
)]
#[case::from_metrics(
    Some("http://host/p/v1/metrics"),
    Signal::Traces,
    Some("http://host/p/v1/traces")
)]
#[case::splunk(
    Some("https://ingest.us0.signalfx.com/v2/trace/otlp"),
    Signal::Traces,
    Some("https://ingest.us0.signalfx.com/v2/trace/otlp")
)]
#[case::splunk_trailing_slash(
    Some("https://ingest.signalfx.com/v2/trace/otlp/"),
    Signal::Traces,
    Some("https://ingest.signalfx.com/v2/trace/otlp")
)]
#[case::metrics_splunk_not_exempt(
    Some("https://h/v2/trace/otlp"),
    Signal::Metrics,
    Some("https://h/v2/trace/otlp/v1/metrics")
)]
#[case::logs_splunk_not_exempt(
    Some("https://h/v2/trace/otlp"),
    Signal::Logs,
    Some("https://h/v2/trace/otlp/v1/logs")
)]
#[case::v1_only(Some("http://host/v1"), Signal::Traces, Some("http://host/v1/traces"))]
#[case::v1_only_trailing_slash(
    Some("http://host/v1/"),
    Signal::Metrics,
    Some("http://host/v1/metrics")
)]
#[case::extra_segment_is_not_swapped(
    Some("http://h/v1/logs/extra"),
    Signal::Traces,
    Some("http://h/v1/logs/extra/v1/traces")
)]
#[case::metrics_base(
    Some("http://host:4318"),
    Signal::Metrics,
    Some("http://host:4318/v1/metrics")
)]
#[case::metrics_correct(
    Some("http://host/v1/metrics"),
    Signal::Metrics,
    Some("http://host/v1/metrics")
)]
#[case::metrics_from_traces(
    Some("http://host/v1/traces"),
    Signal::Metrics,
    Some("http://host/v1/metrics")
)]
#[case::metrics_from_logs(
    Some("http://host/v1/logs/"),
    Signal::Metrics,
    Some("http://host/v1/metrics")
)]
#[case::logs_base(
    Some("http://host:4318/"),
    Signal::Logs,
    Some("http://host:4318/v1/logs")
)]
#[case::logs_correct(Some("http://host/v1/logs"), Signal::Logs, Some("http://host/v1/logs"))]
#[case::logs_from_traces(
    Some("http://host/v1/traces"),
    Signal::Logs,
    Some("http://host/v1/logs")
)]
#[case::logs_from_metrics(
    Some("http://host/p/v1/metrics"),
    Signal::Logs,
    Some("http://host/p/v1/logs")
)]
#[case::logs_extra_segment_is_not_swapped(
    Some("http://h/v1/metrics/extra"),
    Signal::Logs,
    Some("http://h/v1/metrics/extra/v1/logs")
)]
fn endpoint_is_normalized_per_signal(
    #[case] endpoint: Option<&str>,
    #[case] signal: Signal,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        normalize_otel_endpoint(endpoint, signal, None).as_deref(),
        expected
    );
}

#[rstest]
#[case::langtrace_trace_path(
    Some("https://app.langtrace.ai/api/trace"),
    Signal::Traces,
    Some("langtrace"),
    Some("https://app.langtrace.ai/api/trace")
)]
#[case::langtrace_path_other_callback(
    Some("https://app.langtrace.ai/api/trace"),
    Signal::Traces,
    Some("arize"),
    Some("https://app.langtrace.ai/api/trace/v1/traces")
)]
#[case::langtrace_path_no_callback(
    Some("https://app.langtrace.ai/api/trace"),
    Signal::Traces,
    None,
    Some("https://app.langtrace.ai/api/trace/v1/traces")
)]
#[case::langtrace_path_for_logs(
    Some("https://app.langtrace.ai/api/trace"),
    Signal::Logs,
    Some("langtrace"),
    Some("https://app.langtrace.ai/api/trace/v1/logs")
)]
#[case::langtrace_path_for_metrics(
    Some("https://app.langtrace.ai/api/trace"),
    Signal::Metrics,
    Some("langtrace"),
    Some("https://app.langtrace.ai/api/trace/v1/metrics")
)]
fn langtrace_only_keeps_its_trace_path_for_traces(
    #[case] endpoint: Option<&str>,
    #[case] signal: Signal,
    #[case] callback_name: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        normalize_otel_endpoint(endpoint, signal, callback_name).as_deref(),
        expected
    );
}
