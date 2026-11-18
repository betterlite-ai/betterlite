use betterlite_bedrock::embedding_call::EmbeddingCallResponse;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::error::RedactionError;
use serde::Serialize;
use serde_json::Number;
use serde_json::Value;
use serde_json::json;
use thiserror::Error;

use super::fixtures::client::Sent;
use super::fixtures::harness;
use super::input::Case;

#[derive(Debug, Error)]
pub(crate) enum AdapterError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error("the fixture setup failed: {message}")]
    Setup { message: String },
    #[error("the fixture clock is out of range")]
    Clock,
    #[error("an embedding vector holds a value that is not a finite number")]
    Number,
}

impl From<AdapterError> for Box<HelperError> {
    fn from(error: AdapterError) -> Self {
        Self::new(HelperError {
            class: String::from("EmbeddingCallAdapter"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

// LiteLLM's exceptions here carry a status only, so the probe compares the status and leaves the class and the message out
pub(crate) fn probe(case: &Case) -> Result<Value, AdapterError> {
    let harness = harness::run(case)?;
    let (ok, status, response) = match &harness.outcome {
        Ok(outcome) => (true, Value::Null, response_value(outcome)?),
        Err(error) => (false, json!(error.status()), Value::Null),
    };
    Ok(json!({
        "ok": ok,
        "status": status,
        "type": Value::Null,
        "param": Value::Null,
        "code": Value::Null,
        "requests": requests_value(&harness.requests)?,
        "factory": harness.factory,
        "response": response,
        "phases": harness.phases,
        "remaining": harness.remaining,
    }))
}

fn response_value(reply: &EmbeddingCallResponse) -> Result<Value, AdapterError> {
    match reply {
        EmbeddingCallResponse::Batch(reply) => Ok(json!({
            "object": reply.object,
            "data": records(&reply.data, |record| {
                Ok(json!({
                    "object": record.object,
                    "index": record.index,
                    "embedding": vector_value(&record.embedding)?,
                }))
            })?,
            "model": reply.model,
            "usage": usage_value(&reply.usage)?,
            "invocation_arn": reply.invocation_arn,
        })),
        EmbeddingCallResponse::Legacy(reply) => Ok(json!({
            "object": reply.object,
            "data": records(&reply.data, |record| {
                Ok(json!({
                    "object": record.object,
                    "index": record.index,
                    "embedding": embedding_value(&record.embedding)?,
                }))
            })?,
            "model": reply.model,
            "usage": usage_value(&reply.usage)?,
            "invocation_arn": Value::Null,
        })),
    }
}

fn records<Record, RecordValue>(
    records: &[Record],
    value: RecordValue,
) -> Result<Value, AdapterError>
where
    RecordValue: Fn(&Record) -> Result<Value, AdapterError>,
{
    let mut values = Vec::with_capacity(records.len());
    for record in records {
        values.push(value(record)?);
    }
    Ok(Value::Array(values))
}

// LiteLLM dumps every Usage details field with the unset ones as null, and the Rust types name only the set ones
fn usage_value<Usage>(usage: &Usage) -> Result<Value, AdapterError>
where
    Usage: Serialize,
{
    let mut value = serde_json::to_value(usage)?;
    if let Value::Object(fields) = &mut value
        && let Some(Value::Object(details)) = fields.get_mut("prompt_tokens_details")
    {
        details.retain(|_, detail| !detail.is_null());
    }
    Ok(value)
}

// pydantic reads every vector entry as a float, so the numbers compare as floats on both sides
fn vector_value(vector: &EmbeddingVector) -> Result<Value, AdapterError> {
    match vector {
        EmbeddingVector::Base64(encoded) => Ok(Value::String(encoded.clone())),
        EmbeddingVector::Floats(numbers) => floats_value(numbers),
    }
}

fn floats_value(numbers: &[Number]) -> Result<Value, AdapterError> {
    let mut values = Vec::with_capacity(numbers.len());
    for number in numbers {
        values.push(Value::from(number.as_f64().ok_or(AdapterError::Number)?));
    }
    Ok(Value::Array(values))
}

fn embedding_value(embedding: &Json) -> Result<Value, AdapterError> {
    if let Json::List(items) = embedding {
        return records(items, number_value);
    }
    Ok(serde_json::to_value(embedding)?)
}

fn number_value(item: &Json) -> Result<Value, AdapterError> {
    if let Json::Number(number) = item {
        return number.as_f64().map(Value::from).ok_or(AdapterError::Number);
    }
    Ok(serde_json::to_value(item)?)
}

fn requests_value(requests: &[Sent]) -> Result<Value, AdapterError> {
    let mut values = Vec::with_capacity(requests.len());
    for request in requests {
        values.push(json!({
            "url": request.url,
            "headers": headers_value(&request.headers)?,
            "body": serde_json::from_slice::<Value>(&request.body)?,
        }));
    }
    Ok(Value::Array(values))
}

// httpx adds transport headers the fixture client does not, and a SigV4 signature covers the json.dumps text, so it cannot match
const SKIPPED: [&str; 6] = [
    "host",
    "content-length",
    "user-agent",
    "accept-encoding",
    "connection",
    "transfer-encoding",
];

fn headers_value(headers: &[(String, String)]) -> Result<Value, AdapterError> {
    let mut fields = Object::new();
    for (name, value) in headers {
        let lowered = name.to_ascii_lowercase();
        if SKIPPED.contains(&lowered.as_str()) || (lowered == "accept" && value == "*/*") {
            continue;
        }
        fields.insert(lowered, Json::Text(signature_scope(value)));
    }
    Ok(serde_json::to_value(fields)?)
}

fn signature_scope(value: &str) -> String {
    match value.split_once(", Signature=") {
        Some((scope, _signature)) => scope.to_owned(),
        None => value.to_owned(),
    }
}
