use betterlite_logging::otel::integrations::arize_messages::text::coerce_text;
use betterlite_logging::otel::integrations::arize_messages::tool_calls::summarize_tool_calls;
use betterlite_logging::otel::integrations::arize_messages::{
    ArizeOtelAttributes, set_response_attributes,
};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::span_attributes::LLMObsAttributes as _;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SetMessages { messages: OtelModelValue },
    SetResponse { response: OtelModelValue },
    CoerceText { value: OtelModelValue },
    SummarizeToolCalls { tool_calls: Vec<OtelModelValue> },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::SetMessages { messages } => {
                let record = span_attributes(|span| {
                    let mut kwargs = MetadataMap::new();
                    kwargs.insert("messages".to_owned(), messages.clone());
                    ArizeOtelAttributes::set_messages(span, &kwargs);
                })?;
                drop_array_contents(record, messages)
            }
            Input::SetResponse { response } => Json::Object(span_attributes(|span| {
                set_response_attributes(span, response);
            })?),
            Input::CoerceText { value: raw } => json!(coerce_text(raw)),
            Input::SummarizeToolCalls { tool_calls } => {
                let summary = summarize_tool_calls(tool_calls);
                let parsed = serde_json::from_str(&summary).unwrap_or(Json::String(summary));
                normalize_value(parsed, "")
            }
        };
        Ok(json!({ "result": value }))
    }
}

fn span_attributes<F>(record: F) -> Result<Map<String, Json>, Box<crate::helpers::HelperError>>
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("arize-messages").start("test");
    record(&mut span);
    span.end();
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| internal_error(error.to_string()))?;
    let Some(finished) = spans.first() else {
        return Ok(Map::new());
    };
    Ok(finished
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), attribute_json(&pair.value)))
        .collect())
}

fn internal_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InternalError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn attribute_json(value: &Value) -> Json {
    match value {
        Value::Bool(flag) => Json::Bool(*flag),
        Value::I64(number) => Json::from(*number),
        Value::F64(number) => {
            serde_json::Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        Value::String(text) => Json::String(text.to_string()),
        Value::Array(array) => Json::String(array.to_string()),
        _ => Json::String(value.to_string()),
    }
}

fn drop_array_contents(mut record: Map<String, Json>, messages: &OtelModelValue) -> Json {
    let Some(entries) = messages.as_array() else {
        return Json::Object(record);
    };
    let last_has_array_content = entries
        .last()
        .and_then(OtelModelValue::as_object)
        .and_then(|message| message.get("content"))
        .is_some_and(|content| content.as_array().is_some());
    if last_has_array_content {
        record.remove("input.value");
    }
    for (index, message) in entries.iter().enumerate() {
        let has_array_content = message
            .as_object()
            .and_then(|message| message.get("content"))
            .is_some_and(|content| content.as_array().is_some());
        if has_array_content {
            record.remove(&format!("llm.input_messages.{index}.message.content"));
        }
    }
    Json::Object(record)
}

fn normalize_value(value: Json, path: &str) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, entry)| {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    (key, normalize_value(entry, &child))
                })
                .collect(),
        ),
        Json::Array(items) => Json::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| normalize_value(item, &format!("{path}.{index}")))
                .collect(),
        ),
        Json::String(text) if path == "output.value" || path.ends_with(".function.arguments") => {
            match serde_json::from_str(&text) {
                Ok(parsed) => normalize_value(parsed, path),
                Err(_) => Json::String(text),
            }
        }
        other => other,
    }
}
