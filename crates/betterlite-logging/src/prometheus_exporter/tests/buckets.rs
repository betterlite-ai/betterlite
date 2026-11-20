use betterlite_prometheus::collector_registry::metric::Bucket;
use rstest::rstest;

use crate::prometheus_exporter::PrometheusExporterSettings;

use super::{default_label_settings, registered};

fn bounded(seconds: &[f64]) -> Vec<Bucket> {
    seconds
        .iter()
        .copied()
        .map(Bucket::Seconds)
        .chain([Bucket::Infinite])
        .collect()
}

fn latency_default() -> Vec<Bucket> {
    bounded(&[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
        420.0, 600.0,
    ])
}

fn client_default() -> Vec<Bucket> {
    bounded(&[
        0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
    ])
}

fn batch_duration() -> Vec<Bucket> {
    bounded(&[
        60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 14400.0, 28800.0, 43200.0,
        86400.0,
    ])
}

fn registered_buckets(settings: &PrometheusExporterSettings, metric: &str) -> Vec<Bucket> {
    let (registry, _collectors) = registered(settings, &default_label_settings());
    registry
        .get(metric)
        .expect("the collector is registered")
        .description
        .buckets
        .clone()
}

#[rstest]
#[case::request_total_latency("litellm_request_total_latency_metric")]
#[case::llm_api_latency("litellm_llm_api_latency_metric")]
#[case::time_to_first_token("litellm_llm_api_time_to_first_token_metric")]
#[case::overhead_latency("litellm_overhead_latency_metric")]
#[case::overhead_with_guardrails("litellm_overhead_with_guardrails_latency_metric")]
#[case::request_queue_time("litellm_request_queue_time_seconds")]
#[case::guardrail_latency("litellm_guardrail_latency_seconds")]
fn latency_histograms_use_the_latency_buckets_by_default(#[case] metric: &str) {
    let settings = PrometheusExporterSettings::default();
    assert_eq!(registered_buckets(&settings, metric), latency_default());
}

#[test]
fn the_latency_buckets_field_keeps_the_default_table() {
    let (_registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(collectors.latency_buckets, latency_default());
}

#[test]
fn latency_per_output_token_keeps_the_client_default_buckets() {
    let settings = PrometheusExporterSettings::default();
    assert_eq!(
        registered_buckets(&settings, "litellm_deployment_latency_per_output_token"),
        client_default()
    );
    assert_ne!(
        client_default(),
        latency_default(),
        "the client default differs from the latency table"
    );
}

#[test]
fn the_managed_batch_duration_uses_the_batch_duration_buckets() {
    let settings = PrometheusExporterSettings::default();
    assert_eq!(
        registered_buckets(&settings, "litellm_managed_batch_duration_seconds"),
        batch_duration()
    );
}

#[rstest]
#[case::request_total_latency("litellm_request_total_latency_metric")]
#[case::llm_api_latency("litellm_llm_api_latency_metric")]
#[case::time_to_first_token("litellm_llm_api_time_to_first_token_metric")]
#[case::overhead_latency("litellm_overhead_latency_metric")]
#[case::overhead_with_guardrails("litellm_overhead_with_guardrails_latency_metric")]
#[case::request_queue_time("litellm_request_queue_time_seconds")]
#[case::guardrail_latency("litellm_guardrail_latency_seconds")]
fn custom_latency_buckets_replace_the_default_for_latency_histograms(#[case] metric: &str) {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_latency_buckets = Some(vec![0.5_f64, 2.5_f64]);
    assert_eq!(registered_buckets(&settings, metric), bounded(&[0.5, 2.5]));
}

#[test]
fn custom_latency_buckets_leave_the_other_histograms_alone() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_latency_buckets = Some(vec![0.5_f64, 2.5_f64]);
    assert_eq!(
        registered_buckets(&settings, "litellm_deployment_latency_per_output_token"),
        client_default()
    );
    assert_eq!(
        registered_buckets(&settings, "litellm_managed_batch_duration_seconds"),
        batch_duration()
    );
}

#[test]
fn the_latency_buckets_field_keeps_the_configured_list() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_latency_buckets = Some(vec![0.5_f64, 2.5_f64]);
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(
        collectors.latency_buckets,
        vec![Bucket::Seconds(0.5), Bucket::Seconds(2.5)]
    );
}
