use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use crate::cost_usage::error::CostUsageError;
use crate::cost_usage::token_count::TokenCount;

fn modality_field_name(modality: &str) -> Option<&'static str> {
    match modality {
        "text" | "document" => Some("text_tokens"),
        "audio" => Some("audio_tokens"),
        "image" => Some("image_tokens"),
        "video" => Some("video_tokens"),
        _ => None,
    }
}

// Only an actual JSON string selects a field; any other shape resolves to no recognized field.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "RawModalityName")]
struct ModalityName(Option<String>);

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawModalityName {
    Str(Box<str>),
    Other(Box<IgnoredAny>),
}

impl From<RawModalityName> for ModalityName {
    fn from(raw: RawModalityName) -> Self {
        match raw {
            RawModalityName::Str(value) => Self(Some(String::from(value))),
            RawModalityName::Other(_ignored) => Self(None),
        }
    }
}

impl ModalityName {
    fn field_name(&self) -> Option<&'static str> {
        let name = self.0.as_deref()?;
        modality_field_name(&name.to_lowercase())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ModalityEntryRecord {
    modality: ModalityName,
    tokens: TokenCount,
}

// A non-mapping entry is a transformation error, never silently skipped.
pub(crate) fn modality_token_sums(
    entries: &[&RawValue],
) -> Result<BTreeMap<&'static str, i64>, CostUsageError> {
    let mut sums: BTreeMap<&'static str, i64> = BTreeMap::new();
    for entry in entries {
        let record: ModalityEntryRecord = serde_json::from_str(entry.get())
            .map_err(|_error| CostUsageError::ModalityEntryNotObject)?;
        let Some(field) = record.modality.field_name() else {
            continue;
        };
        let sum = sums.entry(field).or_insert(0);
        *sum = sum
            .checked_add(record.tokens.0)
            .ok_or(CostUsageError::ModalitySumOverflow { field })?;
    }
    Ok(sums)
}

mod tests;
