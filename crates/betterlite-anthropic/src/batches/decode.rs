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
        let first = self
            .processing
            .checked_add(self.succeeded)
            .and_then(|value| value.checked_add(self.errored))
            .and_then(|value| value.checked_add(self.canceled));
        match first.and_then(|value| value.checked_add(self.expired)) {
            Some(total) => Ok(total),
            None => Err(DecodeError::CountOverflow),
        }
    }
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Failed to parse Anthropic batch response: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Anthropic batch response {field} {expected}")]
    Shape {
        field: &'static str,
        expected: &'static str,
    },
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
            field: "the body",
            expected: "must be a JSON object",
        });
    };
    let raw_status = match fields.shift_remove("processing_status") {
        None => Some("in_progress".to_owned()),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text),
        Some(_other) => {
            return Err(DecodeError::Shape {
                field: "processing_status",
                expected: "must be a string or null",
            });
        }
    };
    let status = match raw_status.as_deref() {
        Some("ended") => BatchStatus::Completed,
        Some("canceling") => BatchStatus::Cancelling,
        _ => BatchStatus::InProgress,
    };
    let id = match fields.shift_remove("id") {
        None => String::new(),
        Some(Json::Text(text)) => text,
        Some(_other) => {
            return Err(DecodeError::Shape {
                field: "id",
                expected: "must be a string",
            });
        }
    };
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

fn counts_of(value: Option<Json>) -> Result<BatchCounts, DecodeError> {
    let Some(value) = value else {
        return Ok(BatchCounts::default());
    };
    let Json::Object(mut fields) = value else {
        return Err(DecodeError::Shape {
            field: "request_counts",
            expected: "must be a JSON object",
        });
    };
    Ok(BatchCounts {
        processing: count_of(&mut fields, "processing")?,
        succeeded: count_of(&mut fields, "succeeded")?,
        errored: count_of(&mut fields, "errored")?,
        canceled: count_of(&mut fields, "canceled")?,
        expired: count_of(&mut fields, "expired")?,
    })
}

fn count_of(fields: &mut IndexMap<String, Json>, name: &'static str) -> Result<i64, DecodeError> {
    match fields.shift_remove(name) {
        None => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or_else(|| DecodeError::Shape {
            field: name,
            expected: "must be an integer",
        }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(DecodeError::Shape {
                field: name,
                expected: "must be an integer",
            })
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
    for line in text.trim().split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Json>(line) {
            Ok(value) => value,
            // Python skips a row json.loads cannot decode and keeps reading the rest
            Err(error) if matches!(error.classify(), Category::Syntax | Category::Eof) => continue,
            Err(source) => return Err(DecodeError::Parse { source }),
        };
        messages.push(message_of(value)?);
    }
    Ok(messages)
}

fn message_of(value: Json) -> Result<Json, DecodeError> {
    let Json::Object(mut row) = value else {
        return Err(DecodeError::Shape {
            field: "result row",
            expected: "must be a JSON object",
        });
    };
    let Some(Json::Object(mut result)) = row.shift_remove("result") else {
        return Err(DecodeError::Shape {
            field: "result row",
            expected: "must hold a result object",
        });
    };
    result
        .shift_remove("message")
        .ok_or_else(|| DecodeError::Shape {
            field: "result row",
            expected: "must hold a result message",
        })
}
