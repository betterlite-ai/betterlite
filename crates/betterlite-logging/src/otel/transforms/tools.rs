use opentelemetry::trace::Span;

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;

const REQUEST_FUNCTIONS: &str = "llm.request.functions";
const COMPLETIONS: &str = "gen_ai.completion";
// Function.__annotations__ holds exactly these two fields on openai's tool call
const FUNCTION_FIELDS: [&str; 2] = ["arguments", "name"];

pub fn set_tools_attributes<S>(span: &mut S, tools: &[OtelModelValue])
where
    S: Span,
{
    for (index, tool) in tools.iter().enumerate() {
        let Some(function) = tool
            .as_object()
            .and_then(|fields| fields.get("function"))
            .and_then(OtelModelValue::as_object)
        else {
            continue;
        };
        if function.is_empty() {
            continue;
        }
        let prefix = format!("{REQUEST_FUNCTIONS}.{index}");
        let name = function
            .get("name")
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        span.safe_set_attribute(&format!("{prefix}.name"), &name);
        let description = function
            .get("description")
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        span.safe_set_attribute(&format!("{prefix}.description"), &description);
        let parameters = function
            .get("parameters")
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        let parameters_text = serde_json::to_string(&parameters).unwrap_or_default();
        span.safe_set_attribute(
            &format!("{prefix}.parameters"),
            &OtelModelValue::String(parameters_text),
        );
    }
}

#[must_use]
pub fn tool_calls_kv_pair(tool_calls: &[OtelModelValue]) -> Vec<(String, OtelModelValue)> {
    let mut pairs = Vec::new();
    for (index, tool_call) in tool_calls.iter().enumerate() {
        let Some(function) = tool_call
            .as_object()
            .and_then(|fields| fields.get("function"))
            .and_then(OtelModelValue::as_object)
        else {
            continue;
        };
        if function.is_empty() {
            continue;
        }
        for field in FUNCTION_FIELDS {
            let Some(value) = function.get(field).filter(|value| match value {
                OtelModelValue::Null(()) => false,
                OtelModelValue::Bool(flag) => *flag,
                OtelModelValue::Number(number) => {
                    number.as_f64().is_some_and(|amount| amount != 0.0_f64)
                }
                OtelModelValue::String(text) => !text.is_empty(),
                OtelModelValue::Array(items) => !items.is_empty(),
                OtelModelValue::Object(fields) => !fields.is_empty(),
            }) else {
                continue;
            };
            pairs.push((
                format!("{COMPLETIONS}.{index}.function_call.{field}"),
                value.clone(),
            ));
        }
    }
    pairs
}

#[path = "tools/otel_transforms/mod.rs"]
mod otel_transforms;
