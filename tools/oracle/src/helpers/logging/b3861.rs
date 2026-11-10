use betterlite_logging::otel::integrations::arize_attributes::set_attributes;
use betterlite_logging::otel::integrations::arize_messages::ArizeOtelAttributes;
use betterlite_logging::otel::integrations::weave_attributes::set_weave_otel_attributes;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::Value as OtelValue;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const EXCEPTION_EVENT: &str = "exception";
const JSON_ATTRIBUTES: [&str; 4] = [
    "input.value",
    "output.value",
    "metadata",
    "llm.invocation_parameters",
];

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SetAttributes {
        kwargs: MetadataMap,
        response: OtelModelValue,
        #[serde(default)]
        redact_messages: bool,
    },
    SetWeaveAttributes {
        kwargs: MetadataMap,
        response: OtelModelValue,
        #[serde(default)]
        redact_messages: bool,
    },
}

struct Recorded {
    attributes: Map<String, Value>,
    exceptions: Vec<Value>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let kwargs = match input {
            Input::SetAttributes { kwargs, .. } | Input::SetWeaveAttributes { kwargs, .. } => {
                kwargs
            }
        };
        let Recorded {
            attributes: raw_attributes,
            exceptions,
        } = recorded_span(|span| match input {
            Input::SetAttributes {
                response,
                redact_messages,
                ..
            } => {
                set_attributes::<ArizeOtelAttributes, SdkSpan>(
                    span,
                    kwargs,
                    response,
                    *redact_messages,
                );
            }
            Input::SetWeaveAttributes {
                response,
                redact_messages,
                ..
            } => set_weave_otel_attributes(span, kwargs, response, *redact_messages),
        })?;
        let mut attributes = normalize_attributes(raw_attributes);
        drop_array_contents(&mut attributes, kwargs.get("messages"));
        Ok(json!({
            "result": {
                "attributes": Value::Object(attributes),
                "exceptions": exceptions,
            },
        }))
    }
}

fn recorded_span<F>(record: F) -> Result<Recorded, Box<crate::helpers::HelperError>>
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3861").start("test");
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
        .map(|pair| (pair.key.to_string(), attribute_json(&pair.value)))
        .collect();
    let exceptions = finished
        .events
        .events
        .iter()
        .filter(|event| event.name == EXCEPTION_EVENT)
        .map(|event| exception_json(&event.attributes))
        .collect();
    Ok(Recorded {
        attributes,
        exceptions,
    })
}

fn attribute_json(value: &OtelValue) -> Value {
    match value {
        OtelValue::String(text) => Value::String(text.to_string()),
        OtelValue::Bool(flag) => json!(flag),
        OtelValue::I64(number) => json!(number),
        OtelValue::F64(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        OtelValue::Array(_) | _ => Value::String(value.to_string()),
    }
}

fn exception_json(attributes: &[opentelemetry::KeyValue]) -> Value {
    let mut record = Map::new();
    for pair in attributes {
        // the Python adapter reports the exception as {type, message}
        let field = match pair.key.as_str() {
            "exception.type" => "type",
            "exception.message" => "message",
            _ => continue,
        };
        record.insert(field.to_owned(), attribute_json(&pair.value));
    }
    Value::Object(record)
}

fn normalize_attributes(mut attributes: Map<String, Value>) -> Map<String, Value> {
    for key in JSON_ATTRIBUTES {
        let Some(Value::String(text)) = attributes.get(key) else {
            continue;
        };
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            attributes.insert(key.to_owned(), parsed);
        }
    }
    attributes
}

fn drop_array_contents(attributes: &mut Map<String, Value>, messages: Option<&OtelModelValue>) {
    let Some(entries) = messages.and_then(OtelModelValue::as_array) else {
        return;
    };
    if entries.last().is_some_and(has_array_content) {
        attributes.remove("input.value");
    }
    for index in 0..entries.len() {
        if entries.get(index).is_some_and(has_array_content) {
            attributes.remove(&format!("llm.input_messages.{index}.message.content"));
        }
    }
}

fn has_array_content(message: &OtelModelValue) -> bool {
    message
        .as_object()
        .and_then(|fields| fields.get("content"))
        .is_some_and(|content| content.as_array().is_some())
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
