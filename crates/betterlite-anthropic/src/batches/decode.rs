use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;
use serde_json::error::Category;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchBody {
    Lifecycle,
    Result,
}

#[derive(Debug)]
pub enum DecodedBody {
    Lifecycle(Box<LifecycleBatch>),
    Results(Vec<Json>),
}

// The fields of an Anthropic MessageBatch that the OpenAI batch record is built from, before the clock reads them
#[derive(Debug)]
pub struct LifecycleBatch {
    pub id: String,
    pub raw_status: Option<String>,
    pub status: BatchStatus,
    pub created_at: Json,
    pub ended_at: Json,
    pub expires_at: Json,
    pub cancel_initiated_at: Json,
    pub archived_at: Json,
    pub counts: BatchCounts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchCounts {
    pub processing: i64,
    pub succeeded: i64,
    pub errored: i64,
    pub canceled: i64,
    pub expired: i64,
}

impl BatchCounts {
    pub fn total(self) -> Result<i64, DecodeError> {
        self.processing
            .checked_add(self.succeeded)
            .and_then(|value| value.checked_add(self.errored))
            .and_then(|value| value.checked_add(self.canceled))
            .and_then(|value| value.checked_add(self.expired))
            .ok_or(DecodeError::CountOverflow)
    }
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Failed to parse Anthropic batch response: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Anthropic batch response {detail}")]
    Shape { detail: &'static str },
    #[error("the Anthropic batch response {field} must be an integer")]
    CountField { field: &'static str },
    #[error("the Anthropic batch response request_counts total overflows")]
    CountOverflow,
}

pub fn decode_anthropic_batch_result(
    operation: BatchBody,
    body: &[u8],
) -> Result<DecodedBody, DecodeError> {
    match operation {
        BatchBody::Lifecycle => {
            decode_lifecycle(body).map(|batch| DecodedBody::Lifecycle(Box::new(batch)))
        }
        BatchBody::Result => decode_results(body).map(DecodedBody::Results),
    }
}

fn decode_lifecycle(body: &[u8]) -> Result<LifecycleBatch, DecodeError> {
    let value: Json =
        serde_json::from_slice(body).map_err(|source| DecodeError::Parse { source })?;
    let Json::Object(mut fields) = value else {
        return Err(DecodeError::Shape {
            detail: "the body must be a JSON object",
        });
    };
    let raw_status = status_field(fields.shift_remove("processing_status"))?;
    let status = batch_status(raw_status.as_deref());
    let id = id_field(fields.shift_remove("id"))?;
    Ok(LifecycleBatch {
        id,
        raw_status,
        status,
        created_at: fields.shift_remove("created_at").unwrap_or(Json::Null),
        ended_at: fields.shift_remove("ended_at").unwrap_or(Json::Null),
        expires_at: fields.shift_remove("expires_at").unwrap_or(Json::Null),
        cancel_initiated_at: fields
            .shift_remove("cancel_initiated_at")
            .unwrap_or(Json::Null),
        archived_at: fields.shift_remove("archived_at").unwrap_or(Json::Null),
        counts: counts_of(fields.shift_remove("request_counts"))?,
    })
}

fn status_field(value: Option<Json>) -> Result<Option<String>, DecodeError> {
    match value {
        None => Ok(Some("in_progress".to_owned())),
        Some(Json::Null) => Ok(None),
        Some(raw) => text_field(raw, "processing_status must be a string or null").map(Some),
    }
}

fn batch_status(raw_status: Option<&str>) -> BatchStatus {
    match raw_status {
        Some("ended") => BatchStatus::Completed,
        Some("canceling") => BatchStatus::Cancelling,
        _ => BatchStatus::InProgress,
    }
}

fn id_field(value: Option<Json>) -> Result<String, DecodeError> {
    value.map_or_else(
        || Ok(String::new()),
        |raw| text_field(raw, "id must be a string"),
    )
}

fn text_field(value: Json, detail: &'static str) -> Result<String, DecodeError> {
    let Json::Text(text) = value else {
        return Err(DecodeError::Shape { detail });
    };
    Ok(text)
}

fn counts_of(value: Option<Json>) -> Result<BatchCounts, DecodeError> {
    match value {
        None => Ok(BatchCounts::default()),
        Some(Json::Object(mut fields)) => Ok(BatchCounts {
            processing: count_of(&mut fields, "processing")?,
            succeeded: count_of(&mut fields, "succeeded")?,
            errored: count_of(&mut fields, "errored")?,
            canceled: count_of(&mut fields, "canceled")?,
            expired: count_of(&mut fields, "expired")?,
        }),
        Some(_other) => Err(DecodeError::Shape {
            detail: "request_counts must be a JSON object",
        }),
    }
}

fn count_of(fields: &mut IndexMap<String, Json>, name: &'static str) -> Result<i64, DecodeError> {
    match fields.shift_remove(name) {
        None => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(DecodeError::CountField { field: name }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(DecodeError::CountField { field: name })
        }
    }
}

// Python reads the body as text, so each invalid byte sequence shows as one U+FFFD
pub(crate) fn decoded_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

fn decode_results(body: &[u8]) -> Result<Vec<Json>, DecodeError> {
    let text = decoded_text(body);
    let mut messages = Vec::new();
    for row in text.lines() {
        let line = row.trim();
        if line.is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Json>(line) {
            Ok(value) => value,
            Err(error) => {
                // Python skips a row json.loads cannot decode and keeps reading the rest
                if matches!(error.classify(), Category::Syntax | Category::Eof) {
                    continue;
                }
                return Err(DecodeError::Parse { source: error });
            }
        };
        messages.push(message_of(value)?);
    }
    Ok(messages)
}

fn message_of(value: Json) -> Result<Json, DecodeError> {
    let Json::Object(mut row) = value else {
        return Err(DecodeError::Shape {
            detail: "result row must be a JSON object",
        });
    };
    let Some(Json::Object(mut result)) = row.shift_remove("result") else {
        return Err(DecodeError::Shape {
            detail: "result row must hold a result object",
        });
    };
    result.shift_remove("message").ok_or(DecodeError::Shape {
        detail: "result row must hold a result message",
    })
}
