use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::bedrock_embedding_io::response::EmbeddingReply;
use betterlite_bedrock::embedding_call::EmbeddingCallResponse;
use betterlite_bedrock::embedding_call::error::EmbeddingCallError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::cohere_legacy_embedding::decode::{
    LegacyEmbeddingRecord, LegacyEmbeddingResponse,
};
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Json;
use betterlite_redact::error::RedactionError;
use serde::Serialize;
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

pub(crate) fn probe(case: &Case) -> Result<Value, AdapterError> {
    let harness = harness::run(case)?;
    let (ok, status, kind, message, response) = match &harness.outcome {
        Ok(outcome) => (
            true,
            Value::Null,
            Kind::Ok,
            Value::Null,
            response_value(outcome)?,
        ),
        Err(error) => (
            false,
            json!(error.status()),
            kind(error),
            message(error),
            Value::Null,
        ),
    };
    Ok(json!({
        "ok": ok,
        "status": status,
        "kind": kind,
        "message": message,
        "requests": requests_value(&harness.requests)?,
        "factory": harness.factory,
        "response": response,
        "phases": harness.phases,
        "remaining": harness.remaining,
    }))
}

// LiteLLM's handlers differ in which exception class carries a failure, so the class is compared
// while the free text stays with the two generic provider messages
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Ok,
    Http,
    Bedrock,
    Transport,
    Decode,
    Provider,
    Other,
}

fn kind(error: &EmbeddingCallError) -> Kind {
    match error {
        EmbeddingCallError::Embedding(EmbeddingIoError::Provider { .. }) => Kind::Http,
        EmbeddingCallError::Embedding(EmbeddingIoError::Handler(_)) => Kind::Bedrock,
        EmbeddingCallError::Embedding(EmbeddingIoError::Transport(_)) => Kind::Transport,
        EmbeddingCallError::Embedding(
            EmbeddingIoError::Decode { .. }
            | EmbeddingIoError::Params { .. }
            | EmbeddingIoError::Unrepresentable
            | EmbeddingIoError::MalformedEmbedding
            | EmbeddingIoError::NotANumber
            | EmbeddingIoError::IndexOverflow,
        ) => Kind::Decode,
        EmbeddingCallError::Embedding(EmbeddingIoError::Unmapped { .. })
        | EmbeddingCallError::Provider { .. }
        | EmbeddingCallError::Unmapped => Kind::Provider,
        EmbeddingCallError::Cohere(CohereLegacyEmbeddingError::Status(_)) => Kind::Http,
        EmbeddingCallError::Cohere(CohereLegacyEmbeddingError::Transport(_)) => Kind::Transport,
        EmbeddingCallError::Embedding(_)
        | EmbeddingCallError::Endpoint(_)
        | EmbeddingCallError::Signing(_)
        | EmbeddingCallError::Cohere(_)
        | EmbeddingCallError::Json { .. }
        | EmbeddingCallError::Field { .. }
        | EmbeddingCallError::Titan(_)
        | EmbeddingCallError::Nova(_)
        | EmbeddingCallError::Marengo(_)
        | EmbeddingCallError::Logging(_)
        | EmbeddingCallError::Timeout { .. } => Kind::Other,
    }
}

fn message(error: &EmbeddingCallError) -> Value {
    match kind(error) {
        Kind::Provider => Value::String(error.to_string()),
        Kind::Ok | Kind::Http | Kind::Bedrock | Kind::Transport | Kind::Decode | Kind::Other => {
            Value::Null
        }
    }
}

fn response_value(reply: &EmbeddingCallResponse) -> Result<Value, AdapterError> {
    match reply {
        EmbeddingCallResponse::Batch(reply) => batch_value(reply),
        EmbeddingCallResponse::Legacy(reply) => legacy_value(reply),
    }
}

fn batch_value(reply: &EmbeddingReply) -> Result<Value, AdapterError> {
    Ok(json!({
        "object": reply.object,
        "data": batch_records(&reply.data)?,
        "model": reply.model,
        "usage": usage_value(&reply.usage)?,
        "invocation_arn": reply.invocation_arn,
    }))
}

fn legacy_value(reply: &LegacyEmbeddingResponse) -> Result<Value, AdapterError> {
    Ok(json!({
        "object": reply.object,
        "data": legacy_records(&reply.data)?,
        "model": reply.model,
        "usage": usage_value(&reply.usage)?,
        "invocation_arn": Value::Null,
    }))
}

// LiteLLM's Usage model dumps every details field, the unset ones as null, and the Rust types
// name only the fields their family sets, so only set details survive on both sides
fn usage_value(usage: &impl Serialize) -> Result<Value, AdapterError> {
    let mut value = serde_json::to_value(usage)?;
    if let Value::Object(fields) = &mut value
        && let Some(Value::Object(details)) = fields.get_mut("prompt_tokens_details")
    {
        details.retain(|_, detail| !detail.is_null());
    }
    Ok(value)
}

fn batch_records(records: &[EmbeddingRecord]) -> Result<Value, AdapterError> {
    let mut values = Vec::with_capacity(records.len());
    for record in records {
        values.push(json!({
            "object": record.object,
            "index": record.index,
            "embedding": vector_value(&record.embedding)?,
        }));
    }
    Ok(Value::Array(values))
}

// pydantic reads every vector entry as a float, so the numbers compare as floats on both sides
fn vector_value(vector: &EmbeddingVector) -> Result<Value, AdapterError> {
    match vector {
        EmbeddingVector::Base64(encoded) => Ok(Value::String(encoded.clone())),
        EmbeddingVector::Floats(numbers) => numbers
            .iter()
            .map(|number| number.as_f64().map(Value::from).ok_or(AdapterError::Number))
            .collect::<Result<Vec<Value>, AdapterError>>()
            .map(Value::Array),
    }
}

fn legacy_records(records: &[LegacyEmbeddingRecord]) -> Result<Value, AdapterError> {
    let mut values = Vec::with_capacity(records.len());
    for record in records {
        values.push(json!({
            "object": record.object,
            "index": record.index,
            "embedding": legacy_embedding(&record.embedding)?,
        }));
    }
    Ok(Value::Array(values))
}

fn legacy_embedding(embedding: &Json) -> Result<Value, AdapterError> {
    match embedding {
        Json::List(items) => items
            .iter()
            .map(number_value)
            .collect::<Result<Vec<Value>, AdapterError>>()
            .map(Value::Array),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Ok(serde_json::to_value(embedding)?)
        }
    }
}

fn number_value(item: &Json) -> Result<Value, AdapterError> {
    match item {
        Json::Number(number) => number.as_f64().map(Value::from).ok_or(AdapterError::Number),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Ok(serde_json::to_value(item)?)
        }
    }
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

// httpx adds transport headers the fixture client does not, and a SigV4 signature covers the
// json.dumps text rather than the compact bytes, so the signature itself cannot match
const SKIPPED: [&str; 6] = [
    "host",
    "content-length",
    "user-agent",
    "accept-encoding",
    "connection",
    "transfer-encoding",
];

fn headers_value(headers: &[(String, String)]) -> Result<Value, AdapterError> {
    let mut fields = betterlite_prompts::arguments::json::Object::new();
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
