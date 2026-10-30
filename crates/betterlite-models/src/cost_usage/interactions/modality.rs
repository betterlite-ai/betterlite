use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use crate::cost_usage::error::{CostUsageError, ModalityField};
use crate::cost_usage::token_count::token_count_from_raw;

fn modality_field_name(modality: &str) -> Option<ModalityField> {
    match modality {
        "text" | "document" => Some(ModalityField::Text),
        "audio" => Some(ModalityField::Audio),
        "image" => Some(ModalityField::Image),
        "video" => Some(ModalityField::Video),
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
    fn field_name(&self) -> Option<ModalityField> {
        let name = self.0.as_deref()?;
        modality_field_name(&name.to_lowercase())
    }
}

type ModalityEntryFields = BTreeMap<Box<str>, Box<RawValue>>;

// A non-mapping entry is a transformation error, never silently skipped.
pub(crate) fn modality_token_sums(
    entries: &[&RawValue],
) -> Result<BTreeMap<ModalityField, i64>, CostUsageError> {
    let mut sums: BTreeMap<ModalityField, i64> = BTreeMap::new();
    for entry in entries {
        let fields: ModalityEntryFields = serde_json::from_str(entry.get())
            .map_err(|_error| CostUsageError::ModalityEntryNotObject)?;
        let modality: ModalityName = fields
            .get("modality")
            .map_or_else(ModalityName::default, |raw| {
                serde_json::from_str(raw.get()).unwrap_or_default()
            });
        let Some(field) = modality.field_name() else {
            continue;
        };
        let tokens = match fields.get("tokens") {
            Some(raw) => token_count_from_raw(raw)?,
            None => 0,
        };
        let sum = sums.entry(field).or_insert(0);
        *sum = sum
            .checked_add(tokens)
            .ok_or(CostUsageError::ModalitySumOverflow { field })?;
    }
    Ok(sums)
}

// A falsy non-array value (false, 0, "", {}, null) is an empty list; a truthy one is retained for transformation.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub(super) enum ModalityList {
    #[default]
    Empty,
    Entries(Vec<Box<RawValue>>),
    NotArray,
}

impl ModalityList {
    pub(super) fn entries(&self) -> &[Box<RawValue>] {
        match self {
            Self::Empty | Self::NotArray => &[],
            Self::Entries(entries) => entries,
        }
    }
}

impl From<Box<RawValue>> for ModalityList {
    fn from(raw: Box<RawValue>) -> Self {
        if let Ok(entries) = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
            return Self::Entries(entries);
        }
        if is_truthy(&raw) {
            return Self::NotArray;
        }
        Self::Empty
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawJsonShape {
    Null,
    Bool(bool),
    Number(Box<Number>),
    Str(Box<str>),
    Array(Vec<IgnoredAny>),
    Object(BTreeMap<Box<str>, Box<IgnoredAny>>),
}

fn is_truthy(raw: &RawValue) -> bool {
    let shape = serde_json::from_str::<RawJsonShape>(raw.get()).unwrap_or(RawJsonShape::Null);
    match shape {
        RawJsonShape::Null => false,
        RawJsonShape::Bool(flag) => flag,
        RawJsonShape::Number(number) => number_is_truthy(&number),
        RawJsonShape::Str(text) => !text.is_empty(),
        RawJsonShape::Array(entries) => !entries.is_empty(),
        RawJsonShape::Object(fields) => !fields.is_empty(),
    }
}

fn number_is_truthy(number: &Number) -> bool {
    let source = number.to_string();
    let mantissa = source
        .split_once(['e', 'E'])
        .map_or(source.as_str(), |parts| parts.0);
    mantissa
        .chars()
        .any(|character| character.is_ascii_digit() && character != '0')
}
