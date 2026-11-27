use std::str;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::BatchRecordsError;
use crate::batch_embeddings::classify::{BatchRecord, BatchRecordKind, classify_batch_record};

// every boundary Python's str.splitlines breaks on, so a record line ends where Python says it does
pub const LINE_BOUNDARIES: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

const BODY_FIELD: &str = "body";
const CALL_PREFIX: &str = "CALL";
const CUSTOM_ID_FIELD: &str = "custom_id";

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
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut fields = IndexMap::new();
        fields.insert(String::from("recordId"), self.record_id.clone());
        fields.insert(String::from("modelInput"), self.model_input.clone());
        Json::Object(fields)
    }
}

impl OpenAiBatchRecord {
    // `custom_id or f"CALL{str(idx).zfill(7)}"`: a present null stays null.
    #[must_use]
    pub fn record_id(&self, index: usize) -> Json {
        match &self.custom_id {
            CustomId::Missing => Json::Text(format!("{CALL_PREFIX}{index:07}")),
            CustomId::Null => Json::Null,
            CustomId::Text(text) => Json::Text(text.clone()),
        }
    }
}

// One uploaded JSONL line: UTF-8, blank lines skipped, and the OpenAI record read with its kind.
pub fn decode_batch_input_line(
    line: &[u8],
) -> Result<Option<OpenAiBatchRecord>, BatchRecordsError> {
    let text = str::from_utf8(line)?;
    // Python's str.strip also drops U+001F, which Rust's White_Space set leaves out
    let trimmed =
        text.trim_matches(|character: char| character.is_whitespace() || character == '\u{1f}');
    if trimmed.is_empty() {
        return Ok(None);
    }
    // json.loads reads the line as splitlines returns it, so only the four JSON whitespace bytes are skipped.
    let classified: BatchRecord = serde_json::from_str(text).map_err(BatchRecordsError::Json)?;
    let fields: IndexMap<String, Json> =
        serde_json::from_str(text).map_err(BatchRecordsError::Json)?;
    let custom_id = match fields.get(CUSTOM_ID_FIELD) {
        None => CustomId::Missing,
        Some(Json::Null) => CustomId::Null,
        Some(Json::Text(custom)) => CustomId::Text(custom.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(BatchRecordsError::Field {
                field: CUSTOM_ID_FIELD,
            });
        }
    };
    let body = fields
        .get(BODY_FIELD)
        .cloned()
        .unwrap_or_else(|| Json::Object(IndexMap::new()));
    if !matches!(body, Json::Object(_)) {
        return Err(BatchRecordsError::RecordShape);
    }
    Ok(Some(OpenAiBatchRecord {
        kind: classify_batch_record(&classified),
        custom_id,
        body,
    }))
}
