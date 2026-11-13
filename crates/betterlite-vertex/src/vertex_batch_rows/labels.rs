use std::iter::repeat_n;

use betterlite_models::decisions::Json;
use data_encoding::{BASE32, BASE32_NOPAD};
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
    let encoded = BASE32_NOPAD.encode(custom_id.as_bytes()).to_lowercase();
    let letters: Vec<char> = encoded.chars().collect();
    let mut chunks: Vec<&[char]> = letters.chunks(MAX_ENCODED_LEN).collect();
    chunks.resize(chunks.len().max(1), &[]);
    for (index, chunk) in chunks.into_iter().enumerate() {
        let name = match index {
            0 => String::from(RAW_LABEL),
            _ => format!("{NUMBERED_RAW_LABEL_PREFIX}{index}"),
        };
        let payload: String = chunk.iter().collect();
        labels.insert(name, Json::Text(format!("{RAW_LABEL_PREFIX}{payload}")));
    }
}

pub(crate) fn decode_gcp_label_value_chunks(chunks: &[&str]) -> Option<String> {
    let mut payload = String::new();
    for chunk in chunks {
        payload.push_str(chunk.strip_prefix(RAW_LABEL_PREFIX)?);
    }
    // Python pads the payload to eight symbols, rejects a padding a quantum cannot hold, and ignores leftover bits
    let mut padded = payload.to_uppercase();
    let padding = 8_usize.checked_sub(padded.chars().count().checked_rem(8)?)?;
    padded.extend(repeat_n('=', padding.checked_rem(8)?));
    let mut specification = BASE32.specification();
    specification.check_trailing_bits = false;
    let decoded = specification
        .encoding()
        .ok()?
        .decode(padded.as_bytes())
        .ok()?;
    String::from_utf8(decoded).ok()
}

pub fn custom_id_from_labels(labels: &IndexMap<String, Json>) -> Result<String, BatchRowsError> {
    let label = |name: &str| match labels.get(name) {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(BatchRowsError::Label(name.to_owned())),
    };
    let Some(raw) = label(RAW_LABEL)?.filter(|raw| !raw.is_empty()) else {
        return Ok(label(ORDINARY_LABEL)?.unwrap_or(UNKNOWN_ID).to_owned());
    };
    let mut numbered = Vec::new();
    for name in labels.keys() {
        let Some(suffix) = name.strip_prefix(NUMBERED_RAW_LABEL_PREFIX) else {
            continue;
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let number: u64 = suffix
            .parse()
            .map_err(|_error| BatchRowsError::Label(name.clone()))?;
        numbered.push((number, label(name)?.unwrap_or_default()));
    }
    numbered.sort_by_key(|(number, _chunk)| *number);
    let mut chunks = vec![raw];
    chunks.extend(numbered.into_iter().map(|(_number, chunk)| chunk));
    Ok(decode_gcp_label_value_chunks(&chunks).unwrap_or_else(|| raw.to_owned()))
}
