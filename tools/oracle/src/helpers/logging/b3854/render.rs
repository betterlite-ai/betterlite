use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Array, KeyValue, Value as OtelValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span, SpanData};
use serde_json::{Value, json};

pub(super) fn config_json(config: &OpenTelemetryConfig) -> Value {
    json!({
        "exporter": exporter_text(&config.exporter),
        "endpoint": config.endpoint,
        "headers": config.headers,
        "enable_metrics": config.signals.enable_metrics,
        "enable_events": config.signals.enable_events,
        "service_name": config.service_name,
        "deployment_environment": config.deployment_environment,
        "model_id": config.model_id,
        "ignore_context_propagation": config.ignore_context_propagation,
        "skip_set_global": config.skip_set_global,
        "capture_message_content": config.capture_message_content,
        "semconv_stability_opt_in": config
            .semconv_stability_opt_in
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "baggage_team_metadata_keys": config.baggage_team_metadata_keys,
        "baggage_metadata_keys": config.baggage_metadata_keys,
        "attributes": config.attributes,
    })
}

fn exporter_text(exporter: &Exporter) -> &str {
    match exporter {
        Exporter::Console => "console",
        Exporter::InMemory => "in_memory",
        Exporter::Named(text) => text,
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
        "attributes": attributes_json(first),
        "events": events_json(first),
    }))
}

fn attributes_json(span: &SpanData) -> Value {
    Value::Array(
        span.attributes
            .iter()
            .map(|pair| json!([pair.key.to_string(), attribute_json(pair)]))
            .collect(),
    )
}

fn attribute_json(pair: &KeyValue) -> Value {
    if pair.key.as_str() == "litellm.team.metadata"
        && let OtelValue::String(text) = &pair.value
        && let Ok(parsed) = serde_json::from_str(text.as_str())
    {
        return parsed;
    }
    value_json(&pair.value)
}

fn events_json(span: &SpanData) -> Value {
    Value::Array(
        span.events
            .events
            .iter()
            .map(|event| {
                json!({
                    "name": event.name.as_ref(),
                    "attributes": event
                        .attributes
                        .iter()
                        .map(|pair| json!([pair.key.to_string(), value_json(&pair.value)]))
                        .collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn value_json(value: &OtelValue) -> Value {
    match value {
        OtelValue::Bool(flag) => json!(flag),
        OtelValue::I64(number) => json!(number),
        OtelValue::F64(number) => json!(number),
        OtelValue::String(text) => json!(text.as_str()),
        OtelValue::Array(items) => array_json(items),
        _ => Value::Null,
    }
}

fn array_json(array: &Array) -> Value {
    match array {
        Array::Bool(items) => json!(items),
        Array::I64(items) => json!(items),
        Array::F64(items) => json!(items),
        Array::String(items) => Value::Array(
            items
                .iter()
                .map(|item| Value::String(item.as_str().to_owned()))
                .collect(),
        ),
        _ => Value::Null,
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
