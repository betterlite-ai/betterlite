use std::str;

use betterlite_context::cloud_storage::{
    BEDROCK_MANAGED_S3_PREFIXES, build_object_name, sanitize_component,
};
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::ExtractedFileData;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::S3SigningError;

// every boundary Python's str.splitlines breaks on; a CRLF pair leaves an empty piece the blank check drops
const LINE_BOUNDARIES: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

// only the first record's body is read, so a later record may carry any body B648 maps
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UploadRecord {
    #[serde(default = "default_body")]
    pub body: Json,
}

fn default_body() -> Json {
    Json::Object(IndexMap::new())
}

pub fn decode_upload_record(line: &[u8]) -> Result<Option<UploadRecord>, S3SigningError> {
    let text = str::from_utf8(line).map_err(S3SigningError::ContentUtf8)?;
    // Python's str.strip also drops U+001F, which Rust's White_Space set leaves out
    let trimmed =
        text.trim_matches(|character: char| character.is_whitespace() || character == '\u{1f}');
    if trimmed.is_empty() {
        return Ok(None);
    }
    // serde would read a JSON array as the struct's fields in order, but a record is an object
    if trimmed.starts_with('[') {
        return Err(S3SigningError::RecordShape);
    }
    serde_json::from_str(text)
        .map(Some)
        .map_err(S3SigningError::Record)
}

fn record_model(record: UploadRecord) -> Result<String, S3SigningError> {
    let Json::Object(mut body) = record.body else {
        return Err(S3SigningError::RecordShape);
    };
    match body.shift_remove("model") {
        None => Ok(String::new()),
        Some(Json::Text(model)) => Ok(model),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(S3SigningError::RecordShape)
        }
    }
}

pub fn get_object_name(
    extracted: &ExtractedFileData,
    purpose: CreatePurpose,
    id: Uuid,
) -> Result<String, S3SigningError> {
    let [batch_prefix, upload_prefix, _output_prefix] = BEDROCK_MANAGED_S3_PREFIXES;
    if purpose == CreatePurpose::Batch {
        let content = str::from_utf8(&extracted.content).map_err(S3SigningError::ContentUtf8)?;
        let records = content
            .split(LINE_BOUNDARIES)
            .map(|line| decode_upload_record(line.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(first) = records.into_iter().flatten().next() {
            let model = record_model(first)?;
            let hyphenated = model
                .strip_prefix("bedrock/")
                .unwrap_or(&model)
                .replace(':', "-");
            let safe_model = sanitize_component(Some(&hyphenated), "model");
            return Ok(format!(
                "{batch_prefix}{safe_model}-{}.jsonl",
                id.hyphenated()
            ));
        }
    }
    Ok(build_object_name(
        upload_prefix,
        extracted.filename.as_deref(),
        "file",
        id,
    ))
}
