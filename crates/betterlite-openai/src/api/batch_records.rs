use std::str::{Utf8Error, from_utf8};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::batches::records::{BatchEndpoint, CompletionWindow, OutputExpiry};

mod scan;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Error)]
pub enum BatchRecordsError {
    #[error("batch content exceeds the limit of {limit} bytes")]
    JsonTooLarge { limit: usize },
    #[error("a batch row exceeds the limit of {limit} bytes")]
    RowTooLarge { limit: usize },
    #[error("batch content exceeds the limit of {limit} rows")]
    TooManyRows { limit: usize },
    #[error("batch content is not valid UTF-8")]
    NotUtf8(#[from] Utf8Error),
}

// CreateBatchRequest is a total=False TypedDict, so every key may be absent
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct CreateBatchRequest {
    pub completion_window: Option<CompletionWindow>,
    pub endpoint: Option<BatchEndpoint>,
    pub input_file_id: Option<String>,
    pub metadata: Option<IndexMap<String, String>>,
    pub output_expires_after: Option<OutputExpiry>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_body: Option<IndexMap<String, String>>,
    pub timeout: Option<f64>,
}

// serde's flatten buffers numbers badly under arbitrary_precision, so the parent keys are repeated
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct LiteLLMBatchCreateRequest {
    pub completion_window: Option<CompletionWindow>,
    pub endpoint: Option<BatchEndpoint>,
    pub input_file_id: Option<String>,
    pub metadata: Option<IndexMap<String, String>>,
    pub output_expires_after: Option<OutputExpiry>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_body: Option<IndexMap<String, String>>,
    pub timeout: Option<f64>,
    pub model: Option<String>,
    pub disable_fallbacks: Option<bool>,
}

// RetrieveBatchRequest and CancelBatchRequest are the same four keys
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct BatchIdRequest {
    pub batch_id: Option<String>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_body: Option<IndexMap<String, String>>,
    pub timeout: Option<f64>,
}

pub type RetrieveBatchRequest = BatchIdRequest;
pub type CancelBatchRequest = BatchIdRequest;

// `after: str | NotGiven`, where NotGiven is an absent key
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ListBatchRequest {
    pub after: Option<String>,
}

// A row the model rewrite reaches: a JSON object whose body is an object, every other key kept as read
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchRequestLine {
    #[serde(flatten)]
    pub fields: Map<String, Value>,
    pub body: Map<String, Value>,
}

// Any other well-formed row is carried as read, because LiteLLM writes it back unchanged
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum BatchRow {
    Request(BatchRequestLine),
    Other(Value),
}

impl From<Value> for BatchRow {
    fn from(value: Value) -> Self {
        let Value::Object(mut fields) = value else {
            return Self::Other(value);
        };
        match fields.remove("body") {
            Some(Value::Object(body)) => Self::Request(BatchRequestLine { fields, body }),
            Some(other) => {
                fields.insert("body".to_owned(), other);
                Self::Other(Value::Object(fields))
            }
            None => Self::Other(Value::Object(fields)),
        }
    }
}

impl BatchRow {
    #[must_use]
    pub fn with_model(self, new_model_name: &str) -> Self {
        match self {
            Self::Request(mut line) => {
                line.body
                    .insert("model".to_owned(), Value::String(new_model_name.to_owned()));
                Self::Request(line)
            }
            Self::Other(value) => Self::Other(value),
        }
    }
}

// The start of a buffer that never parsed, cut to the first 100 characters like LiteLLM's log line
#[derive(Debug)]
pub struct UnparsedTail {
    pub preview: String,
    pub error: serde_json::Error,
}

#[derive(Debug)]
pub struct DecodedRows {
    pub rows: Vec<BatchRow>,
    pub unparsed: Option<UnparsedTail>,
}

pub fn decode_batch_records(
    bytes: &[u8],
    limits: &WireDecodeLimits,
) -> Result<DecodedRows, BatchRecordsError> {
    if bytes.len() > limits.max_json_bytes {
        return Err(BatchRecordsError::JsonTooLarge {
            limit: limits.max_json_bytes,
        });
    }
    scan::scan_rows(from_utf8(bytes)?, limits)
}
