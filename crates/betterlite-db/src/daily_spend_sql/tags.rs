use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use std::str::{Utf8Error, from_utf8};

const MAX_TAGS_BYTES: usize = 0x0010_0000;
const MAX_TAGS_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum DailyTagsError {
    #[error("request_tags payload is {len} bytes, over the {MAX_TAGS_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("request_tags payload is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("request_tags payload exceeds JSON depth {MAX_TAGS_DEPTH}")]
    TooDeep,
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
    let text = from_utf8(bytes).map_err(DailyTagsError::InvalidUtf8)?;
    check_depth(text)?;
    let tags: Vec<String> = serde_json::from_str(text).map_err(DailyTagsError::Json)?;
    let unique = deduplicate_tags(tags);
    Ok(unique)
}

pub(super) fn tags_from_payload(payload: TagsPayload) -> Result<Vec<String>, DailyTagsError> {
    // A JSON string holds JSON text of the array, matching pydantic's validate_json fallback.
    let tags = match payload {
        TagsPayload::Array(tags) => tags,
        TagsPayload::Text(text) => parse_guarded(text.as_bytes())?,
    };
    Ok(deduplicate_tags(tags))
}

fn deduplicate_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for tag in tags {
        if seen.insert(tag.clone()) {
            unique.push(tag);
        }
    }
    unique
}

fn parse_guarded<T>(bytes: &[u8]) -> Result<T, DailyTagsError>
where
    T: DeserializeOwned,
{
    if bytes.len() > MAX_TAGS_BYTES {
        return Err(DailyTagsError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(DailyTagsError::InvalidUtf8)?;
    check_depth(text)?;
    serde_json::from_str(text).map_err(DailyTagsError::Json)
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

fn advance_depth_scan(
    state: ScanState,
    byte: u8,
    depth: u8,
) -> Result<(ScanState, u8), DailyTagsError> {
    match (state, byte) {
        (ScanState::Structural, b'{' | b'[') => {
            let next_depth = depth.saturating_add(1);
            if usize::from(next_depth) > MAX_TAGS_DEPTH {
                return Err(DailyTagsError::TooDeep);
            }
            Ok((ScanState::Structural, next_depth))
        }
        (ScanState::Structural, b'}' | b']') => {
            Ok((ScanState::Structural, depth.saturating_sub(1)))
        }
        (ScanState::Structural, b'"') | (ScanState::Escaped, _) => Ok((ScanState::Quoted, depth)),
        (ScanState::Quoted, b'\\') => Ok((ScanState::Escaped, depth)),
        (ScanState::Quoted, b'"') => Ok((ScanState::Structural, depth)),
        (ScanState::Structural | ScanState::Quoted, _) => Ok((state, depth)),
    }
}

fn check_depth(text: &str) -> Result<(), DailyTagsError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        (state, depth) = advance_depth_scan(state, byte, depth)?;
    }
    Ok(())
}
