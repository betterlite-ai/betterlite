use betterlite_logging::otel::mappers::base::AttributeMap;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Array, StringValue, Value as OtelValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};
use serde_json::{Value, json};

pub(super) type RecordedSpan = (AttributeMap, Vec<(String, AttributeMap)>);

pub(super) fn attributes_record(attributes: &AttributeMap) -> Value {
    Value::Object(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), json_value(value)))
            .collect(),
    )
}

pub(super) fn recorded_span<F>(record: F) -> Result<RecordedSpan, Box<crate::helpers::HelperError>>
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3846").start("test");
    record(&mut span);
    span.end();
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| helper_error("SpanExport", error.to_string()))?;
    let Some(finished) = spans.first() else {
        return Err(helper_error(
            "SpanExport",
            "the recorded span was not exported".to_owned(),
        ));
    };
    let attributes = finished
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    let events = finished
        .events
        .events
        .iter()
        .map(|event| {
            (
                event.name.to_string(),
                event
                    .attributes
                    .iter()
                    .map(|pair| (pair.key.to_string(), pair.value.clone()))
                    .collect(),
            )
        })
        .collect();
    Ok((attributes, events))
}

pub(super) fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn json_value(value: &OtelValue) -> Value {
    match value {
        OtelValue::Bool(flag) => json!(flag),
        OtelValue::I64(integer) => json!(integer),
        OtelValue::F64(float) => json!(float),
        OtelValue::String(text) => {
            serde_json::from_str(text.as_str()).unwrap_or_else(|_error| json!(text.as_str()))
        }
        OtelValue::Array(array) => array_record(array),
        _ => Value::Null,
    }
}

fn array_record(array: &Array) -> Value {
    match array {
        Array::Bool(flags) => json!(flags),
        Array::I64(integers) => json!(integers),
        Array::F64(floats) => json!(floats),
        Array::String(texts) => json!(texts.iter().map(StringValue::as_str).collect::<Vec<_>>()),
        _ => Value::Null,
    }
}
