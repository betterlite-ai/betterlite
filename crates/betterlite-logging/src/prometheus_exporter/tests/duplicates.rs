use betterlite_prometheus::caller_identity_logic::CallerIdentityError;
use betterlite_prometheus::collector_factory::config::PrometheusMetricsConfig;
use betterlite_prometheus::collector_factory::error::CollectorFactoryError;
use betterlite_prometheus::collector_registry::metric::BucketError;
use betterlite_prometheus::collector_registry::{CollectorRegistry, CollectorRegistryError};

use crate::prometheus_exporter::{
    PrometheusCollectors, PrometheusExporterError, PrometheusExporterSettings,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::{default_label_settings, registered};

const LATENCY_METRICS: [&str; 7] = [
    "litellm_request_total_latency_metric",
    "litellm_llm_api_latency_metric",
    "litellm_llm_api_time_to_first_token_metric",
    "litellm_overhead_latency_metric",
    "litellm_overhead_with_guardrails_latency_metric",
    "litellm_request_queue_time_seconds",
    "litellm_guardrail_latency_seconds",
];

fn registration_error(
    settings: &PrometheusExporterSettings,
    labels: &PrometheusLabelSettings,
    registry: &CollectorRegistry,
) -> PrometheusExporterError {
    PrometheusCollectors::register(settings, labels, registry).expect_err("registration fails")
}

#[test]
fn an_invalid_caller_identity_registers_nothing() {
    let mut labels = default_label_settings();
    labels.caller_identity = "openid".to_owned();
    let registry = CollectorRegistry::default();
    let error = registration_error(&PrometheusExporterSettings::default(), &labels, &registry);
    assert!(registry.descriptions().is_empty());
    assert_eq!(
        error.to_string(),
        "Invalid prometheus_deployment_and_latency_caller_identity=\"openid\". Accepted values: api_key_alias, user_email, both."
    );
    assert!(matches!(
        &error,
        PrometheusExporterError::CallerIdentity(CallerIdentityError::InvalidMode { .. })
    ));
}

#[test]
fn a_second_register_names_the_sample_names_that_clash() {
    let settings = PrometheusExporterSettings::default();
    let labels = default_label_settings();
    let (registry, _collectors) = registered(&settings, &labels);
    let error = registration_error(&settings, &labels, &registry);
    assert_eq!(registry.descriptions().len(), 81);
    match error {
        PrometheusExporterError::Registry(CollectorRegistryError::DuplicatedTimeseries {
            names,
        }) => assert_eq!(
            names,
            [
                "litellm_proxy_failed_requests_metric".to_owned(),
                "litellm_proxy_failed_requests_metric_total".to_owned(),
                "litellm_proxy_failed_requests_metric_created".to_owned(),
            ]
        ),
        other @ (PrometheusExporterError::CallerIdentity(_)
        | PrometheusExporterError::Factory(_)
        | PrometheusExporterError::Registry(_)
        | PrometheusExporterError::Buckets(_)) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn an_unsupported_include_label_reports_the_caller_identity_mode() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: vec![PrometheusMetricsConfig {
            group: "experiments".to_owned(),
            metrics: vec!["litellm_spend_metric".to_owned()],
            include_labels: vec!["not_a_label".to_owned()],
        }],
        prometheus_exclude_metrics: Vec::new(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: None,
        prometheus_emit_input_sequence_length_label: false,
    };
    let registry = CollectorRegistry::default();
    let error = registration_error(&settings, &default_label_settings(), &registry);
    match error {
        PrometheusExporterError::Factory(CollectorFactoryError::Configuration { results }) => {
            assert_eq!(
                results.label_errors[0].caller_identity_mode,
                "api_key_alias"
            );
        }
        other @ (PrometheusExporterError::CallerIdentity(_)
        | PrometheusExporterError::Factory(_)
        | PrometheusExporterError::Registry(_)
        | PrometheusExporterError::Buckets(_)) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn an_empty_latency_bucket_list_fails_at_the_first_latency_histogram() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: Vec::new(),
        prometheus_exclude_metrics: Vec::new(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: Some(Vec::new()),
        prometheus_emit_input_sequence_length_label: false,
    };
    let registry = CollectorRegistry::default();
    let error = registration_error(&settings, &default_label_settings(), &registry);
    assert_eq!(registry.descriptions().len(), 2);
    assert_eq!(error.to_string(), "Must have at least two buckets");
    assert!(matches!(
        &error,
        PrometheusExporterError::Buckets(BucketError::TooFew)
    ));
}

#[test]
fn unsorted_latency_buckets_fail_before_any_histogram_is_registered() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: Vec::new(),
        prometheus_exclude_metrics: Vec::new(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: Some(vec![2.5_f64, 0.5_f64]),
        prometheus_emit_input_sequence_length_label: false,
    };
    let registry = CollectorRegistry::default();
    let error = registration_error(&settings, &default_label_settings(), &registry);
    assert_eq!(registry.descriptions().len(), 2);
    assert_eq!(error.to_string(), "Buckets not in sorted order");
    assert!(matches!(
        &error,
        PrometheusExporterError::Buckets(BucketError::NotSorted)
    ));
}

#[test]
fn a_nan_latency_bucket_is_not_sorted() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: Vec::new(),
        prometheus_exclude_metrics: Vec::new(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: Some(vec![f64::NAN, 1.0_f64]),
        prometheus_emit_input_sequence_length_label: false,
    };
    let registry = CollectorRegistry::default();
    let error = registration_error(&settings, &default_label_settings(), &registry);
    assert!(matches!(
        &error,
        PrometheusExporterError::Buckets(BucketError::NotSorted)
    ));
}

#[test]
fn excluding_every_latency_histogram_skips_bucket_validation() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: Vec::new(),
        prometheus_exclude_metrics: LATENCY_METRICS
            .iter()
            .map(|metric| (*metric).to_owned())
            .collect(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: Some(Vec::new()),
        prometheus_emit_input_sequence_length_label: false,
    };
    let registry = CollectorRegistry::default();
    let collectors =
        PrometheusCollectors::register(&settings, &default_label_settings(), &registry)
            .expect("registration succeeds");
    assert_eq!(registry.descriptions().len(), 74);
    assert!(
        collectors
            .exclude_metrics
            .contains("litellm_guardrail_latency_seconds")
    );
}
