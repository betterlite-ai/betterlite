use betterlite_bedrock::embedding_jobs::{EmbeddingJobsError, StatusWireRequest};
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::fixture::Execution;

const HEADER_NAMES: [&str; 4] = [
    "content-type",
    "x-amz-date",
    "x-amz-security-token",
    "authorization",
];

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    value
        .as_object()
        .and_then(|object| object.get(name))
        .unwrap_or(&Json::Null)
}

fn error_text(error: &EmbeddingJobsError) -> Value {
    match error {
        EmbeddingJobsError::HttpFailure { .. } => Value::String(error.to_string()),
        _ => Value::String(String::new()),
    }
}

// the Python and Rust decoders word their own errors, so only the category is compared
const fn error_kind(error: &EmbeddingJobsError) -> &'static str {
    match error {
        EmbeddingJobsError::HttpFailure { .. } => "status",
        EmbeddingJobsError::Decode { .. } => "decode",
        _ => "other",
    }
}

fn request_value(request: &StatusWireRequest) -> Value {
    let headers: Vec<Value> = request
        .headers
        .iter()
        .filter(|(name, _)| HEADER_NAMES.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| json!([name.to_ascii_lowercase(), value]))
        .collect();
    json!({"url": request.url, "headers": headers, "body": ""})
}

fn pre_value(snapshot: &Json) -> Value {
    let args = field(snapshot, "additional_args");
    let names: Vec<&String> = field(args, "headers")
        .as_object()
        .map(|headers| headers.keys().collect())
        .unwrap_or_default();
    json!({
        "input": field(snapshot, "input"),
        "api_base": field(args, "api_base"),
        "header_names": names,
        "request_str": field(args, "request_str"),
        "complete_input_dict": field(args, "complete_input_dict"),
    })
}

fn post_value(snapshot: &Json) -> Value {
    json!({
        "input": field(snapshot, "input"),
        "original_response": field(snapshot, "original_response"),
        "complete_input_dict": field(field(snapshot, "additional_args"), "complete_input_dict"),
    })
}

pub(crate) fn render(execution: &Execution) -> Value {
    let (kind, value, error, error_kind) = match &execution.result {
        Ok(parsed) => (
            "returned",
            serde_json::to_value(parsed).unwrap_or(Value::Null),
            Value::Null,
            Value::Null,
        ),
        Err(error) => (
            "failed",
            Value::Null,
            error_text(error),
            json!(error_kind(error)),
        ),
    };
    let requests: Vec<Value> = execution.requests.iter().map(request_value).collect();
    let factory: Vec<Value> = execution
        .factory_calls
        .iter()
        .map(|(provider, timeout)| {
            json!({
                "provider": provider,
                "timeout": if timeout.is_some() { json!("override") } else { Value::Null },
            })
        })
        .collect();
    let pre = execution.snapshots.first().map(pre_value);
    let post = execution.snapshots.get(1).map(post_value);
    json!({
        "kind": kind,
        "value": value,
        "error": error,
        "error_kind": error_kind,
        "requests": requests,
        "factory": factory,
        "resolved_regions": &execution.resolved_regions,
        "pre": pre,
        "post": post,
    })
}
