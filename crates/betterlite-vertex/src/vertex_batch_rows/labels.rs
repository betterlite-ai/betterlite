use std::iter::repeat_n;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::vertex_batch_rows::error::BatchRowsError;

pub const GCP_LABEL_VALUE_MAX_LEN: usize = 63;
pub const RAW_LABEL_PREFIX: &str = "b32_";
pub const RAW_LABEL: &str = "litellm_custom_id_raw";
pub const NUMBERED_RAW_LABEL_PREFIX: &str = "litellm_custom_id_raw_";
pub const ORDINARY_LABEL: &str = "litellm_custom_id";
pub const UNKNOWN_ID: &str = "unknown";

const MAX_ENCODED_LEN: usize = GCP_LABEL_VALUE_MAX_LEN - RAW_LABEL_PREFIX.len();

#[must_use]
pub fn sanitize_gcp_label_value(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'a'..='z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .take(GCP_LABEL_VALUE_MAX_LEN)
        .collect()
}

#[must_use]
pub fn encode_gcp_label_value_chunks(value: &str) -> Vec<String> {
    let encoded = data_encoding::BASE32_NOPAD
        .encode(value.as_bytes())
        .to_lowercase();
    let characters: Vec<char> = encoded.chars().collect();
    let chunks: Vec<String> = characters
        .chunks(MAX_ENCODED_LEN)
        .map(|chunk| {
            let mut label = String::from(RAW_LABEL_PREFIX);
            label.extend(chunk);
            label
        })
        .collect();
    if chunks.is_empty() {
        return vec![String::from(RAW_LABEL_PREFIX)];
    }
    chunks
}

pub fn set_custom_id_labels(labels: &mut IndexMap<String, Json>, custom_id: &str) {
    let lossy = sanitize_gcp_label_value(custom_id);
    labels.insert(String::from(ORDINARY_LABEL), Json::Text(lossy));
    let chunks = encode_gcp_label_value_chunks(custom_id);
    if let Some((first, rest)) = chunks.split_first() {
        labels.insert(String::from(RAW_LABEL), Json::Text(first.clone()));
        for (offset, chunk) in rest.iter().enumerate() {
            let number = offset.saturating_add(1);
            labels.insert(
                format!("{NUMBERED_RAW_LABEL_PREFIX}{number}"),
                Json::Text(chunk.clone()),
            );
        }
    }
}

#[must_use]
pub(crate) fn decode_gcp_label_value_chunks(chunks: &[&str]) -> Option<String> {
    let mut payload = String::new();
    for chunk in chunks {
        payload.push_str(chunk.strip_prefix(RAW_LABEL_PREFIX)?);
    }
    decode_base32_text(&payload)
}

pub fn custom_id_from_labels(labels: &IndexMap<String, Json>) -> Result<String, BatchRowsError> {
    if labels.is_empty() {
        return Ok(String::from(UNKNOWN_ID));
    }
    let Some(raw) = labels.get(RAW_LABEL) else {
        return ordinary_label(labels);
    };
    let Json::Text(raw_label) = raw else {
        return Err(BatchRowsError::Label {
            label: String::from(RAW_LABEL),
        });
    };
    if raw_label.is_empty() {
        return ordinary_label(labels);
    }
    let mut chunks: Vec<&str> = vec![raw_label.as_str()];
    let mut numbered: Vec<(u64, &str)> = Vec::new();
    for (name, value) in labels {
        let Some(suffix) = name.strip_prefix(NUMBERED_RAW_LABEL_PREFIX) else {
            continue;
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let index = suffix
            .parse::<u64>()
            .map_err(|_error| BatchRowsError::ChunkIndex {
                label: name.clone(),
            })?;
        let Json::Text(chunk) = value else {
            return Err(BatchRowsError::Label {
                label: name.clone(),
            });
        };
        numbered.push((index, chunk.as_str()));
    }
    numbered.sort_by_key(|(index, _chunk)| *index);
    chunks.extend(numbered.iter().map(|(_index, chunk)| *chunk));
    decode_gcp_label_value_chunks(&chunks).map_or_else(|| Ok(raw_label.clone()), Ok)
}

fn ordinary_label(labels: &IndexMap<String, Json>) -> Result<String, BatchRowsError> {
    match labels.get(ORDINARY_LABEL) {
        None => Ok(String::from(UNKNOWN_ID)),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchRowsError::Label {
                label: String::from(ORDINARY_LABEL),
            })
        }
    }
}

// LiteLLM pads the payload to a multiple of eight and Python then accepts only five of the eight remainders
pub(crate) fn decode_base32_text(encoded: &str) -> Option<String> {
    let mut characters: Vec<char> = encoded.to_uppercase().chars().collect();
    let remainder = characters.len().checked_rem(8)?;
    let padding = 8_usize.checked_sub(remainder)?.checked_rem(8)?;
    characters.extend(repeat_n('=', padding));
    let trailing = characters
        .iter()
        .rev()
        .take_while(|character| **character == '=')
        .count();
    if !matches!(trailing, 0 | 1 | 3 | 4 | 6) {
        return None;
    }
    let symbols = characters.len().checked_sub(trailing)?;
    let symbol_remainder = symbols.checked_rem(8)?;
    let required = 8_usize.checked_sub(symbol_remainder)?.checked_rem(8)?;
    if trailing != required {
        return None;
    }
    let payload: String = characters.get(..symbols)?.iter().collect();
    let mut specification = data_encoding::BASE32_NOPAD.specification();
    specification.check_trailing_bits = false;
    let encoding = specification.encoding().ok()?;
    let decoded = encoding.decode(payload.as_bytes()).ok()?;
    String::from_utf8(decoded).ok()
}
