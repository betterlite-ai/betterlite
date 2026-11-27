use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use super::error::BatchRecordsError;
use crate::batch_embeddings::classify::{BatchRecord, BatchRecordKind, classify_batch_record};

// every boundary Python's str.splitlines breaks on, so a record line ends where Python says it does
pub const LINE_BOUNDARIES: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

const CALL_PREFIX: &str = "CALL";

// One line of an OpenAI batch file, as the JSONL transformer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiBatchRecord {
    pub kind: BatchRecordKind,
    pub custom_id: CustomId,
    pub body: Json,
}

// `.get("custom_id", default)` tells an absent key from a key set to null.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CustomId {
    #[default]
    Missing,
    Null,
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockBatchRecord {
    pub record_id: Json,
    pub model_input: Json,
}

impl BedrockBatchRecord {
    // {"recordId": ..., "modelInput": ...}: the insertion order Python's dict literal gives.
    pub fn to_json(&self) -> Json {
        let mut fields = IndexMap::new();
        fields.insert(String::from("recordId"), self.record_id.clone());
        fields.insert(String::from("modelInput"), self.model_input.clone());
        Json::Object(fields)
    }
}

impl OpenAiBatchRecord {
    // `custom_id or f"CALL{str(idx).zfill(7)}"`: a present null stays null.
    pub fn record_id(&self, index: usize) -> Json {
        match &self.custom_id {
            CustomId::Missing => Json::Text(format!("{CALL_PREFIX}{index:07}")),
            CustomId::Null => Json::Null,
            CustomId::Text(text) => Json::Text(text.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRecord {
    #[serde(default, deserialize_with = "custom_id_value")]
    custom_id: CustomId,
    #[serde(default, deserialize_with = "body_value")]
    body: Option<Json>,
}

// The field is only deserialized when present, so a missing key keeps CustomId::Missing.
fn custom_id_value<'de, D>(deserializer: D) -> Result<CustomId, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.map_or(CustomId::Null, CustomId::Text))
}

// An explicit null is a body that is not an object, not an absent one.
fn body_value<'de, D>(deserializer: D) -> Result<Option<Json>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Json::deserialize(deserializer)?))
}

// One uploaded JSONL line: UTF-8, blank lines skipped, and the OpenAI record read with its kind.
pub fn decode_batch_input_line(
    line: &[u8],
) -> Result<Option<OpenAiBatchRecord>, BatchRecordsError> {
    let text = std::str::from_utf8(line)?;
    // Python's str.strip also drops U+001F, which Rust's White_Space set leaves out
    let trimmed =
        text.trim_matches(|character: char| character.is_whitespace() || character == '\u{1f}');
    if trimmed.is_empty() {
        return Ok(None);
    }
    let classified: BatchRecord = serde_json::from_str(trimmed).map_err(BatchRecordsError::Json)?;
    let raw: RawRecord = serde_json::from_str(trimmed).map_err(BatchRecordsError::Json)?;
    let body = raw.body.unwrap_or_else(|| Json::Object(IndexMap::new()));
    if !matches!(body, Json::Object(_)) {
        return Err(BatchRecordsError::RecordShape);
    }
    Ok(Some(OpenAiBatchRecord {
        kind: classify_batch_record(&classified),
        custom_id: raw.custom_id,
        body,
    }))
}
