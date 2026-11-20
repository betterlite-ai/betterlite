use betterlite_prometheus::collector_factory::config::PrometheusMetricsConfig;

use crate::prometheus_exporter::PrometheusExporterSettings;
use crate::prometheus_label_schema::get_labels;

use super::{default_label_settings, description, registered};

#[test]
fn the_label_cache_holds_the_register_time_sets() {
    let (_registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(collectors.cached_labels.len(), 64);
    assert_eq!(
        collectors
            .cached_labels
            .get("litellm_request_queue_time_seconds"),
        Some(&description(&collectors.litellm_request_queue_time_metric).label_names)
    );
    assert!(!collectors.cached_labels.contains_key("litellm_total_users"));
    assert!(
        !collectors
            .cached_labels
            .contains_key("litellm_guardrail_errors_total")
    );
}

#[test]
fn toggles_after_register_do_not_change_the_registered_labelnames() {
    let settings = PrometheusExporterSettings::default();
    let mut labels = default_label_settings();
    let (_registry, collectors) = registered(&settings, &labels);
    let registered_labels = description(&collectors.litellm_proxy_total_requests_metric)
        .label_names
        .clone();
    assert!(!registered_labels.iter().any(|label| label == "stream"));
    labels.emit_stream_label = true;
    let toggled =
        get_labels("litellm_proxy_total_requests_metric", &labels).expect("labels resolve");
    assert!(toggled.iter().any(|label| label == "stream"));
    assert_eq!(
        collectors.cached_labels["litellm_proxy_total_requests_metric"],
        registered_labels
    );
    assert!(
        !collectors.cached_labels["litellm_proxy_total_requests_metric"]
            .iter()
            .any(|label| label == "stream")
    );
}

#[test]
fn the_emit_flag_controls_the_input_sequence_length_label() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_emit_input_sequence_length_label = true;
    let mut labels = default_label_settings();
    labels.budget_and_length.emit_input_sequence_length = true;
    let (_registry, collectors) = registered(&settings, &labels);
    assert!(collectors.emit_input_sequence_length_label);
    assert!(
        description(&collectors.litellm_request_total_latency_metric)
            .label_names
            .iter()
            .any(|label| label == "input_sequence_length")
    );
}

#[test]
fn the_emit_flag_stays_off_by_default() {
    let (_registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert!(!collectors.emit_input_sequence_length_label);
    assert!(
        !description(&collectors.litellm_request_total_latency_metric)
            .label_names
            .iter()
            .any(|label| label == "input_sequence_length")
    );
}

#[test]
fn the_cache_drops_excluded_labels_and_keeps_the_include_filter() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_metrics_config = vec![PrometheusMetricsConfig {
        group: "experiments".to_owned(),
        metrics: vec!["litellm_cache_hits_metric".to_owned()],
        include_labels: vec!["model".to_owned(), "api_key_alias".to_owned()],
    }];
    settings.prometheus_exclude_labels = vec!["model".to_owned()];
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(
        collectors.cached_labels["litellm_cache_hits_metric"],
        ["api_key_alias"]
    );
    assert!(
        !collectors.cached_labels["litellm_spend_metric"]
            .iter()
            .any(|label| label == "model")
    );
}
