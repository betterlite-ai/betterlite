use serde::Deserialize;
use std::collections::HashSet;

const MAX_TAGS_BYTES: usize = 0x0010_0000;

#[derive(Debug, thiserror::Error)]
pub enum DailyTagsError {
    #[error("request_tags payload is {len} bytes, over the {MAX_TAGS_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("request_tags payload is invalid JSON or not an array of strings")]
    Json(#[source] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum TagsPayload {
    Array(Vec<String>),
    Text(String),
}

pub fn decode_daily_tags(bytes: &[u8]) -> Result<Vec<String>, DailyTagsError> {
    if bytes.len() > MAX_TAGS_BYTES {
        return Err(DailyTagsError::TooLarge { len: bytes.len() });
    }
    let tags: Vec<String> = serde_json::from_slice(bytes).map_err(DailyTagsError::Json)?;
    tags_from_payload(TagsPayload::Array(tags))
}

pub(super) fn tags_from_payload(payload: TagsPayload) -> Result<Vec<String>, DailyTagsError> {
    let tags = match payload {
        TagsPayload::Array(tags) => tags,
        TagsPayload::Text(text) => return decode_daily_tags(text.as_bytes()),
    };
    let mut seen = HashSet::new();
    Ok(tags
        .into_iter()
        .filter(|tag| seen.insert(tag.clone()))
        .collect())
}
