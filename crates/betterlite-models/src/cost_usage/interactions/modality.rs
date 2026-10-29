use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
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

/// A `modality` value read leniently: only an actual JSON string selects a field,
/// any other JSON shape (including a missing key) resolves to no recognized field.
#[derive(Debug, Clone, Default)]
struct ModalityName(Option<String>);

impl<'de> Deserialize<'de> for ModalityName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ModalityNameVisitor)
    }
}

struct ModalityNameVisitor;

impl<'de> Visitor<'de> for ModalityNameVisitor {
    type Value = ModalityName;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a modality name")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(ModalityName(Some(v.to_owned())))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(ModalityName(Some(v)))
    }

    fn visit_bool<E>(self, _v: bool) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_i64<E>(self, _v: i64) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_u64<E>(self, _v: u64) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_f64<E>(self, _v: f64) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(ModalityName(None))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(ModalityName(None))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ModalityName(None))
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

pub(crate) fn as_refs(entries: Option<&Vec<Box<RawValue>>>) -> Vec<&RawValue> {
    entries.into_iter().flatten().map(Box::as_ref).collect()
}

/// Sums token counts per recognized modality field. A field stays present once any
/// entry recognizes it, even when the resulting sum is zero. A non-mapping entry is
/// a transformation error rather than a silently skipped one.
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
