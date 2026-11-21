use betterlite_logging::prometheus_exporter::{PrometheusCollectors, PrometheusExporterSettings};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::collector_factory::config::PrometheusMetricsConfig;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::{Bucket, CollectorDescription};
use serde_json::{Value, json};

mod tests;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    prometheus_metrics_config: Vec<MetricsConfigInput>,
    #[serde(default)]
    prometheus_exclude_metrics: Vec<String>,
    #[serde(default)]
    prometheus_exclude_labels: Vec<String>,
    #[serde(default)]
    prometheus_latency_buckets: Option<Vec<f64>>,
    #[serde(default)]
    prometheus_emit_input_sequence_length_label: bool, // ast-grep-ignore: no-long-names
    caller_identity: String,
}

#[derive(serde::Deserialize)]
struct MetricsConfigInput {
    #[serde(default)]
    group: String,
    metrics: Vec<String>,
    #[serde(default)]
    include_labels: Option<Vec<String>>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let emit_input_length = input.prometheus_emit_input_sequence_length_label; // ast-grep-ignore: no-long-names
        let registry = CollectorRegistry::default();
        let _collectors = PrometheusCollectors::register(
            &exporter_settings(input, emit_input_length),
            &label_settings(input, emit_input_length),
            &registry,
        )
        .or_value_error()?;
        Ok(listing(&registry))
    }
}

fn exporter_settings(input: &Input, emit_input_length: bool) -> PrometheusExporterSettings {
    let configs: Vec<PrometheusMetricsConfig> = input
        .prometheus_metrics_config
        .iter()
        .map(metrics_config)
        .collect();
    PrometheusExporterSettings {
        prometheus_metrics_config: configs,
        prometheus_exclude_metrics: input.prometheus_exclude_metrics.clone(),
        prometheus_exclude_labels: input.prometheus_exclude_labels.clone(),
        prometheus_latency_buckets: input.prometheus_latency_buckets.clone(),
        prometheus_emit_input_sequence_length_label: emit_input_length, // ast-grep-ignore: no-long-names
    }
}

fn metrics_config(config: &MetricsConfigInput) -> PrometheusMetricsConfig {
    PrometheusMetricsConfig {
        group: config.group.clone(),
        metrics: config.metrics.clone(),
        include_labels: config.include_labels.clone().unwrap_or_default(),
    }
}

fn label_settings(input: &Input, emit_input_length: bool) -> PrometheusLabelSettings {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.caller_identity.clone_from(&input.caller_identity);
    settings.budget_and_length.emit_input_sequence_length = emit_input_length;
    settings
}

fn listing(registry: &CollectorRegistry) -> Value {
    let collectors: Vec<Value> = registry
        .descriptions()
        .iter()
        .map(description_value)
        .collect();
    json!({ "collectors": collectors })
}

fn description_value(description: &CollectorDescription) -> Value {
    let buckets: Vec<Value> = description.buckets.iter().map(bucket_value).collect();
    json!({
        "name": description.name,
        "kind": description.kind.as_str(),
        "help": description.help,
        "labelnames": description.label_names,
        "buckets": buckets,
    })
}

fn bucket_value(bucket: &Bucket) -> Value {
    match bucket {
        Bucket::Seconds(seconds) => json!(seconds),
        Bucket::Infinite => json!("+Inf"),
    }
}
