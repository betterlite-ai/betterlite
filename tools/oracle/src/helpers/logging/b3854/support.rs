use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::config::{
    Exporter, MetadataKeys, OpenTelemetryConfig, normalize_metadata_keys,
};
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span, SpanData};
use serde_json::{Value, json};

pub(super) fn normalized(
    config: &OpenTelemetryConfig,
    environment: &BTreeMap<String, String>,
) -> OpenTelemetryConfig {
    let mut resolved = config.clone();
    resolved.normalize(environment);
    resolved
}

pub(super) fn config_json(config: &OpenTelemetryConfig) -> Value {
    json!({
        "exporter": match &config.exporter {
            Exporter::Console => "console",
            Exporter::InMemory => "in_memory",
            Exporter::Named(text) => text,
        },
        "service_name": config.service_name,
        "endpoint": config.endpoint,
        "enable_metrics": config.signals.enable_metrics,
        "headers": config.headers,
        "enable_events": config.signals.enable_events,
        "model_id": config.model_id,
        "deployment_environment": config.deployment_environment,
        "capture_message_content": config.capture_message_content,
        "skip_set_global": config.skip_set_global,
        "ignore_context_propagation": config.ignore_context_propagation,
        "baggage_metadata_keys": config.baggage_metadata_keys,
        "semconv_stability_opt_in": config.semconv_stability_opt_in.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "baggage_team_metadata_keys": config.baggage_team_metadata_keys,
        "attributes": config.attributes,
    })
}

pub(super) fn metadata_keys(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => normalize_metadata_keys(MetadataKeys::Text(text)),
        Value::Array(items) => {
            let entries: Vec<String> = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            normalize_metadata_keys(MetadataKeys::List(&entries))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => Vec::new(),
    }
}

pub(super) fn record<F>(write: F) -> crate::helpers::HelperResult
where
    F: FnOnce(&mut Span),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3854").start("case");
    write(&mut span);
    span.end();
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| invalid(&error.to_string()))?;
    let first = spans.first().ok_or_else(|| invalid("no exported span"))?;
    Ok(json!({
        "attributes": pairs_json(&first.attributes),
        "events": events_json(first),
    }))
}

fn pairs_json(attributes: &[KeyValue]) -> Value {
    Value::Array(
        attributes
            .iter()
            .map(|pair| json!([pair.key.to_string(), pair.value.to_string()]))
            .collect(),
    )
}

fn events_json(span: &SpanData) -> Value {
    Value::Array(
        span.events
            .events
            .iter()
            .map(|event| {
                json!({
                    "name": event.name.as_ref(),
                    "attributes": pairs_json(&event.attributes),
                })
            })
            .collect(),
    )
}

pub(super) fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
