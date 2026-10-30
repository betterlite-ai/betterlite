use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cost_usage::error::CostUsageError;
use crate::cost_usage::token_count::token_count_from_raw;

type GoogleSearchEntryFields = BTreeMap<Box<str>, Box<RawValue>>;

fn entry_type(fields: &GoogleSearchEntryFields) -> Option<String> {
    let raw = fields.get("type")?;
    serde_json::from_str::<String>(raw.get()).ok()
}

// Skips a non-mapping or non-matching entry rather than erroring on it.
pub(crate) fn google_search_query_count(entries: &[&RawValue]) -> Result<i64, CostUsageError> {
    let mut total = 0_i64;
    for entry in entries {
        let Ok(fields) = serde_json::from_str::<GoogleSearchEntryFields>(entry.get()) else {
            continue;
        };
        if entry_type(&fields).as_deref() != Some("google_search") {
            continue;
        }
        let count = match fields.get("count") {
            Some(raw) => token_count_from_raw(raw)?,
            None => 0,
        };
        total = total
            .checked_add(count)
            .ok_or(CostUsageError::SearchCountOverflow)?;
    }
    Ok(total)
}

// A non-array grounding_tool_count contributes zero searches, mirroring LiteLLM's Sequence guard.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub(super) enum GroundingToolCount {
    #[default]
    Empty,
    Entries(Vec<Box<RawValue>>),
}

impl GroundingToolCount {
    pub(super) fn entries(&self) -> &[Box<RawValue>] {
        match self {
            Self::Empty => &[],
            Self::Entries(entries) => entries,
        }
    }
}

impl From<Box<RawValue>> for GroundingToolCount {
    fn from(raw: Box<RawValue>) -> Self {
        match serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
            Ok(entries) => Self::Entries(entries),
            Err(_error) => Self::Empty,
        }
    }
}

mod tests;
