use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_filter_actions::error::ContentFilterActionError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use serde_json::{Value, json};

fn detection(item: &ContentFilterDetection) -> Value {
    match item {
        ContentFilterDetection::Pattern(record) => {
            json!({"type": "pattern", "pattern_name": record.pattern_name})
        }
        ContentFilterDetection::BlockedWord(_) => json!({"type": "blocked_word"}),
        ContentFilterDetection::CategoryKeyword(_) => json!({"type": "category_keyword"}),
        ContentFilterDetection::CompetitorIntent(_) => json!({"type": "competitor_intent"}),
    }
}

fn tool_call(call: &GuardrailValue) -> Value {
    let GuardrailValue::Object(entries) = call else {
        return value_or_null(Some(call));
    };
    let Some(GuardrailValue::Object(function)) = entries.get("function") else {
        return value_or_null(Some(call));
    };
    let id = value_or_null(entries.get("id"));
    let kind = value_or_null(entries.get("type"));
    let name = value_or_null(function.get("name"));
    json!({
        "id": id,
        "type": kind,
        "function": {
            "name": name,
            "arguments": function.get("arguments").map_or(Value::Null, parsed_arguments),
        },
    })
}

// The Python helper reads the argument text as JSON too, so a rewritten tree compares as a tree
fn parsed_arguments(arguments: &GuardrailValue) -> Value {
    let GuardrailValue::Text(text) = arguments else {
        return value_or_null(Some(arguments));
    };
    serde_json::from_str::<Value>(text).unwrap_or_else(|_error| Value::String(text.clone()))
}

pub(super) fn output(
    texts: &[String],
    detections: &[ContentFilterDetection],
    tool_calls: &[GuardrailValue],
    request_data: &BTreeMap<String, GuardrailValue>,
    error: Value,
) -> Value {
    let mcp_arguments = value_or_null(request_data.get("mcp_arguments"));
    let modified_arguments = value_or_null(request_data.get("modified_arguments"));
    json!({
        "texts": Value::Array(texts.iter().map(|text| Value::String(text.clone())).collect()),
        "detections": Value::Array(detections.iter().map(detection).collect()),
        "tool_calls": Value::Array(tool_calls.iter().map(tool_call).collect()),
        "request_data": {
            "mcp_arguments": mcp_arguments,
            "modified_arguments": modified_arguments,
        },
        "error": error,
    })
}

pub(super) fn failure(failure: &GuardrailFailure) -> Value {
    match failure {
        GuardrailFailure::HttpObject { status, detail } => {
            let detail_value = Value::Object(
                detail
                    .iter()
                    .map(|(key, entry)| (key.clone(), value_or_null(Some(entry))))
                    .collect(),
            );
            json!({"kind": "http", "status": status, "detail": detail_value})
        }
        GuardrailFailure::ModifyResponse(modified) => json!({
            "kind": "passthrough",
            "message": modified.message,
            "model": modified.model,
            "guardrail_name": modified.guardrail_name,
        }),
        GuardrailFailure::Http { status, detail } => {
            json!({"kind": "http", "status": status, "detail": detail})
        }
        GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_) => {
            json!({"kind": "other", "message": failure.to_string()})
        }
    }
}

pub(super) fn action_failure(error: &ContentFilterActionError) -> Value {
    let ContentFilterActionError::Blocked { detail } = error else {
        return json!({"kind": "other", "message": error.to_string()});
    };
    json!({"kind": "http", "status": 400, "detail": detail})
}

fn value_or_null(entry: Option<&GuardrailValue>) -> Value {
    let Some(entry) = entry else {
        return Value::Null;
    };
    serde_json::to_value(entry).unwrap_or(Value::Null)
}
