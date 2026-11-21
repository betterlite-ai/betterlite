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
        0.005_f64, 0.01_f64, 0.025_f64, 0.05_f64, 0.1_f64, 0.25_f64, 0.5_f64, 1.0_f64, 2.0_f64,
        5.0_f64, 10.0_f64, 30.0_f64, 60.0_f64, 120.0_f64, 300.0_f64, 420.0_f64, 600.0_f64,
    ])
}

fn client_default() -> Vec<Bucket> {
    bounded(&[
        0.005_f64, 0.01_f64, 0.025_f64, 0.05_f64, 0.075_f64, 0.1_f64, 0.25_f64, 0.5_f64, 0.75_f64,
        1.0_f64, 2.5_f64, 5.0_f64, 7.5_f64, 10.0_f64,
    ])
}

fn batch_duration() -> Vec<Bucket> {
    bounded(&[
        60.0_f64,
        120.0_f64,
        300.0_f64,
        600.0_f64,
        900.0_f64,
        1800.0_f64,
        3600.0_f64,
        7200.0_f64,
        14400.0_f64,
        28800.0_f64,
        43200.0_f64,
        86400.0_f64,
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
    let settings = PrometheusExporterSettings {
        prometheus_latency_buckets: Some(vec![0.5_f64, 2.5_f64]),
        ..PrometheusExporterSettings::default()
    };
    assert_eq!(
        registered_buckets(&settings, metric),
        bounded(&[0.5_f64, 2.5_f64])
    );
}

#[test]
fn custom_latency_buckets_leave_the_other_histograms_alone() {
    let settings = PrometheusExporterSettings {
        prometheus_latency_buckets: Some(vec![0.5_f64, 2.5_f64]),
        ..PrometheusExporterSettings::default()
    };
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
    let settings = PrometheusExporterSettings {
        prometheus_latency_buckets: Some(vec![0.5_f64, 2.5_f64]),
        ..PrometheusExporterSettings::default()
    };
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(
        collectors.latency_buckets,
        vec![Bucket::Seconds(0.5), Bucket::Seconds(2.5)]
    );
}
