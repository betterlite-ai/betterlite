use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::embedding_jobs::transport::StatusWireRequest;
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
    match value {
        Json::Object(fields) => fields.get(name).unwrap_or(&Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &Json::Null,
    }
}

fn error_text(error: &EmbeddingJobsError) -> Value {
    match error {
        EmbeddingJobsError::HttpFailure { .. } => Value::String(error.to_string()),
        EmbeddingJobsError::Credentials(_)
        | EmbeddingJobsError::MissingCredentials
        | EmbeddingJobsError::Signing(_)
        | EmbeddingJobsError::Transport(_)
        | EmbeddingJobsError::Logging(_)
        | EmbeddingJobsError::Endpoint { .. }
        | EmbeddingJobsError::Decode { .. } => Value::String(String::new()),
    }
}

// the Python and Rust decoders word their own errors, so only the category is compared
const fn error_kind(error: &EmbeddingJobsError) -> &'static str {
    match error {
        EmbeddingJobsError::HttpFailure { .. } => "status",
        EmbeddingJobsError::Decode { .. } => "decode",
        EmbeddingJobsError::Credentials(_)
        | EmbeddingJobsError::MissingCredentials
        | EmbeddingJobsError::Signing(_)
        | EmbeddingJobsError::Transport(_)
        | EmbeddingJobsError::Logging(_)
        | EmbeddingJobsError::Endpoint { .. } => "other",
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
    let mut names: Vec<&String> = match field(args, "headers") {
        Json::Object(headers) => headers.keys().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Vec::new(),
    };
    names.sort_unstable();
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

fn outcome(result: &Result<Json, EmbeddingJobsError>) -> (&'static str, Value, Value, Value) {
    match result {
        Ok(parsed) => ("returned", json!(parsed), Value::Null, Value::Null),
        Err(error) => (
            "failed",
            Value::Null,
            error_text(error),
            json!(error_kind(error)),
        ),
    }
}

pub(crate) fn render(execution: &Execution) -> Value {
    let (kind, value, error, error_kind) = outcome(&execution.result);
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
