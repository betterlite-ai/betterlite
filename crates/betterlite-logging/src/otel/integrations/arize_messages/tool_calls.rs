use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;
use serde::Serialize;

use super::keys;

#[derive(Clone, Debug, Serialize)]
pub struct NormalizedToolCall {
    pub id: Option<OtelModelValue>,
    #[serde(rename = "type")]
    pub kind: OtelModelValue,
    pub function: NormalizedToolCallFunction,
}

#[derive(Clone, Debug, Serialize)]
pub struct NormalizedToolCallFunction {
    pub name: Option<OtelModelValue>,
    pub arguments: Option<OtelModelValue>,
}

#[derive(Serialize)]
struct ToolCallSummary<'calls> {
    tool_calls: &'calls [NormalizedToolCall],
}

#[must_use]
pub fn normalize_tool_call(raw: &OtelModelValue) -> Option<NormalizedToolCall> {
    let fields = raw.as_object()?;
    let function = fields.get("function").and_then(OtelModelValue::as_object);
    let name = function
        .and_then(|function_fields| function_fields.get("name"))
        .cloned();
    let arguments = function
        .and_then(|function_fields| function_fields.get("arguments"))
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
        .map(|raw_arguments| match raw_arguments {
            OtelModelValue::String(text) => OtelModelValue::String(text.clone()),
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_) => {
                OtelModelValue::String(serde_json::to_string(raw_arguments).unwrap_or_default())
            }
        });
    Some(NormalizedToolCall {
        id: fields.get("id").cloned(),
        kind: fields
            .get("type")
            .cloned()
            .unwrap_or_else(|| OtelModelValue::String("function".to_owned())),
        function: NormalizedToolCallFunction { name, arguments },
    })
}

#[must_use]
pub fn summarize_tool_calls(tool_calls: &[OtelModelValue]) -> String {
    let normalized = tool_calls
        .iter()
        .filter_map(normalize_tool_call)
        .collect::<Vec<_>>();
    serde_json::to_string(&ToolCallSummary {
        tool_calls: &normalized,
    })
    .unwrap_or_default()
}

pub(super) fn emit_message_tool_calls<S>(span: &mut S, prefix: &str, message: &MetadataMap)
where
    S: Span,
{
    let found = message
        .get("tool_calls")
        .and_then(OtelModelValue::as_array)
        .filter(|calls| !calls.is_empty());
    let Some(tool_calls) = found else {
        return;
    };
    for (call_index, raw) in tool_calls.iter().enumerate() {
        let Some(call) = normalize_tool_call(raw) else {
            continue;
        };
        let call_prefix = format!("{prefix}.{}.{call_index}", keys::MESSAGE_TOOL_CALLS);
        emit_tool_call(span, &call_prefix, &call);
    }
}

fn emit_tool_call<S>(span: &mut S, call_prefix: &str, call: &NormalizedToolCall)
where
    S: Span,
{
    let id = call
        .id
        .as_ref()
        .and_then(OtelModelValue::as_str)
        .filter(|id| !id.is_empty());
    if let Some(found_id) = id {
        span.safe_set_attribute(
            &format!("{call_prefix}.{}", keys::TOOL_CALL_ID),
            &OtelModelValue::String(found_id.to_owned()),
        );
    }
    let name = call
        .function
        .name
        .as_ref()
        .and_then(OtelModelValue::as_str)
        .filter(|name| !name.is_empty());
    if let Some(found_name) = name {
        span.safe_set_attribute(
            &format!("{call_prefix}.{}", keys::TOOL_CALL_FUNCTION_NAME),
            &OtelModelValue::String(found_name.to_owned()),
        );
    }
    if let Some(arguments) = &call.function.arguments {
        span.safe_set_attribute(
            &format!("{call_prefix}.{}", keys::TOOL_CALL_FUNCTION_ARGUMENTS),
            arguments,
        );
    }
}
