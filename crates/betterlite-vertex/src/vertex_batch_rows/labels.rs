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

const MAX_ENCODED_LEN: usize = GCP_LABEL_VALUE_MAX_LEN.saturating_sub(RAW_LABEL_PREFIX.len());

pub fn set_custom_id_labels(labels: &mut IndexMap<String, Json>, custom_id: &str) {
    let lossy = custom_id
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'a'..='z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .take(GCP_LABEL_VALUE_MAX_LEN)
        .collect();
    labels.insert(String::from(ORDINARY_LABEL), Json::Text(lossy));
    let encoded = data_encoding::BASE32_NOPAD
        .encode(custom_id.as_bytes())
        .to_lowercase();
    let characters: Vec<char> = encoded.chars().collect();
    let chunks: Vec<String> = if characters.is_empty() {
        vec![String::from(RAW_LABEL_PREFIX)]
    } else {
        characters
            .chunks(MAX_ENCODED_LEN)
            .map(|chunk| format!("{RAW_LABEL_PREFIX}{}", chunk.iter().collect::<String>()))
            .collect()
    };
    let Some((first, rest)) = chunks.split_first() else {
        return;
    };
    labels.insert(String::from(RAW_LABEL), Json::Text(first.clone()));
    for (offset, chunk) in rest.iter().enumerate() {
        let name = format!("{NUMBERED_RAW_LABEL_PREFIX}{}", offset.saturating_add(1));
        labels.insert(name, Json::Text(chunk.clone()));
    }
}

pub(crate) fn decode_gcp_label_value_chunks(chunks: &[&str]) -> Option<String> {
    let mut payload = String::new();
    for chunk in chunks {
        payload.push_str(chunk.strip_prefix(RAW_LABEL_PREFIX)?);
    }
    decode_base32_text(&payload)
}

pub fn custom_id_from_labels(labels: &IndexMap<String, Json>) -> Result<String, BatchRowsError> {
    let raw = match labels.get(RAW_LABEL) {
        Some(Json::Text(raw)) if !raw.is_empty() => raw,
        Some(Json::Text(_)) | None => return ordinary_label(labels),
        Some(_) => {
            return Err(BatchRowsError::Label {
                label: String::from(RAW_LABEL),
            });
        }
    };
    let mut chunks: Vec<&str> = vec![raw.as_str()];
    let mut numbered: Vec<(u64, &str)> = Vec::new();
    for (name, value) in labels {
        let Some(suffix) = name.strip_prefix(NUMBERED_RAW_LABEL_PREFIX) else {
            continue;
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let number = suffix.parse().map_err(|_error| BatchRowsError::Label {
            label: name.clone(),
        })?;
        let Json::Text(chunk) = value else {
            return Err(BatchRowsError::Label {
                label: name.clone(),
            });
        };
        numbered.push((number, chunk.as_str()));
    }
    numbered.sort_by_key(|(number, _chunk)| *number);
    chunks.extend(numbered.iter().map(|(_number, chunk)| *chunk));
    decode_gcp_label_value_chunks(&chunks).map_or_else(|| Ok(raw.clone()), Ok)
}

fn ordinary_label(labels: &IndexMap<String, Json>) -> Result<String, BatchRowsError> {
    match labels.get(ORDINARY_LABEL) {
        None => Ok(String::from(UNKNOWN_ID)),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(BatchRowsError::Label {
            label: String::from(ORDINARY_LABEL),
        }),
    }
}

// Python pads the payload to eight symbols, rejects a padding a quantum cannot hold, and ignores leftover bits
pub(crate) fn decode_base32_text(encoded: &str) -> Option<String> {
    let mut padded = encoded.to_uppercase();
    let remainder = padded.chars().count().checked_rem(8)?;
    let padding = 8_usize.checked_sub(remainder)?.checked_rem(8)?;
    padded.extend(repeat_n('=', padding));
    let mut specification = data_encoding::BASE32.specification();
    specification.check_trailing_bits = false;
    let encoding = specification.encoding().ok()?;
    let decoded = encoding.decode(padded.as_bytes()).ok()?;
    String::from_utf8(decoded).ok()
}
