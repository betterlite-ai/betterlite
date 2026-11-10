use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_attributes::context::AttributeContext;
use betterlite_logging::otel::legacy_policy::capture::CaptureMode;
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Array, Value as OtelValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};
use serde_json::{Map, Value};

use super::input::Input;

pub(super) fn config(input: &Input) -> OpenTelemetryConfig {
    let mut config = OpenTelemetryConfig::from_env(&BTreeMap::new());
    if input.semconv {
        config
            .semconv_stability_opt_in
            .insert(OTELSemconvCategory::GenAiLatestExperimental);
    }
    config
        .baggage_metadata_keys
        .clone_from(&input.baggage_metadata_keys);
    config
        .baggage_team_metadata_keys
        .clone_from(&input.baggage_team_metadata_keys);
    config
}

pub(super) const fn context<'request>(
    config: &'request OpenTelemetryConfig,
    name: Option<&'request str>,
    capture_mode: CaptureMode,
) -> AttributeContext<'request> {
    AttributeContext {
        callback_name: name,
        config,
        capture_mode,
    }
}

pub(super) fn capture_mode(name: Option<&str>) -> CaptureMode {
    match name {
        Some("SPAN_ONLY") => CaptureMode::SpanOnly,
        Some("EVENT_ONLY") => CaptureMode::EventOnly,
        Some("NO_CONTENT") => CaptureMode::NoContent,
        _ => CaptureMode::SpanAndEvent,
    }
}

pub(super) fn record<F, R>(
    write: F,
) -> Result<(Map<String, Value>, R), Box<crate::helpers::HelperError>>
where
    F: FnOnce(&mut SdkSpan) -> R,
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3856").start("case");
    let result = write(&mut span);
    span.end();
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| invalid(&error.to_string()))?;
    let finished = spans.first().ok_or_else(|| invalid("no exported span"))?;
    let mut attributes = Map::new();
    for pair in &finished.attributes {
        let key = pair.key.to_string();
        // OTLP forbids repeated keys, and LiteLLM's dict keeps one value per key
        if attributes
            .insert(key.clone(), attribute_json(&pair.value))
            .is_some()
        {
            return Err(invalid(&format!("the exported span repeats the key {key}")));
        }
    }
    Ok((attributes, result))
}

fn attribute_json(value: &OtelValue) -> Value {
    match value {
        // the string attributes carry JSON text where a container was written; both sides read it back
        OtelValue::String(text) => {
            serde_json::from_str(text.as_str()).unwrap_or_else(|_| Value::String(text.to_string()))
        }
        OtelValue::Bool(flag) => Value::Bool(*flag),
        OtelValue::I64(number) => Value::Number(serde_json::Number::from(*number)),
        OtelValue::F64(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        OtelValue::Array(array) => Value::Array(array_json(array)),
        _ => Value::String(value.to_string()),
    }
}

fn array_json(array: &Array) -> Vec<Value> {
    match array {
        Array::Bool(values) => values.iter().map(|value| Value::Bool(*value)).collect(),
        Array::I64(values) => values
            .iter()
            .map(|value| Value::Number(serde_json::Number::from(*value)))
            .collect(),
        Array::F64(values) => values
            .iter()
            .map(|value| serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number))
            .collect(),
        Array::String(values) => values
            .iter()
            .map(|value| Value::String(value.to_string()))
            .collect(),
        _ => Vec::new(),
    }
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
