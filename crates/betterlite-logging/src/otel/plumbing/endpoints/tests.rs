#![cfg(test)]
use super::signal_endpoint;
use crate::otel::config::Signal;
use rstest::rstest;

#[rstest]
#[case::absent(None, Signal::Traces, None)]
#[case::empty(Some(""), Signal::Logs, Some(""))]
#[case::traces_base(
    Some("http://host:4318"),
    Signal::Traces,
    Some("http://host:4318/v1/traces")
)]
#[case::traces_slash(
    Some("http://host:4318///"),
    Signal::Traces,
    Some("http://host:4318/v1/traces")
)]
#[case::traces_correct(
    Some("http://host/v1/traces/"),
    Signal::Traces,
    Some("http://host/v1/traces")
)]
#[case::traces_from_logs(
    Some("http://host/v1/logs"),
    Signal::Traces,
    Some("http://host/v1/traces")
)]
#[case::traces_from_metrics(
    Some("http://host/p/v1/metrics"),
    Signal::Traces,
    Some("http://host/p/v1/traces")
)]
#[case::traces_splunk(
    Some("https://ingest.us0.signalfx.com/v2/trace/otlp"),
    Signal::Traces,
    Some("https://ingest.us0.signalfx.com/v2/trace/otlp")
)]
#[case::traces_splunk_slash(
    Some("https://ingest.signalfx.com/v2/trace/otlp/"),
    Signal::Traces,
    Some("https://ingest.signalfx.com/v2/trace/otlp")
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
    Some("http://host/v1/logs"),
    Signal::Metrics,
    Some("http://host/v1/metrics")
)]
#[case::metrics_splunk_not_exempt(
    Some("https://h/v2/trace/otlp"),
    Signal::Metrics,
    Some("https://h/v2/trace/otlp/v1/metrics")
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
    Some("http://host/v1/metrics/"),
    Signal::Logs,
    Some("http://host/v1/logs")
)]
#[case::logs_splunk_not_exempt(
    Some("https://h/v2/trace/otlp"),
    Signal::Logs,
    Some("https://h/v2/trace/otlp/v1/logs")
)]
fn otlp_signal_endpoint_is_rewritten_per_signal(
    #[case] endpoint: Option<&str>,
    #[case] signal: Signal,
    #[case] expected: Option<&str>,
) {
    assert_eq!(signal_endpoint(endpoint, signal).as_deref(), expected);
}
