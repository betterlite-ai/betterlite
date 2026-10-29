use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cost_usage::error::CostUsageError;
use crate::cost_usage::token_count::TokenCount;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GoogleSearchEntry {
    #[serde(rename = "type")]
    kind: Option<String>,
    count: TokenCount,
}

// Skips a non-mapping or non-matching entry rather than erroring on it.
pub(crate) fn google_search_query_count(entries: &[&RawValue]) -> Result<i64, CostUsageError> {
    let mut total = 0_i64;
    for entry in entries {
        let Ok(record) = serde_json::from_str::<GoogleSearchEntry>(entry.get()) else {
            continue;
        };
        if record.kind.as_deref() != Some("google_search") {
            continue;
        }
        total = total
            .checked_add(record.count.0)
            .ok_or(CostUsageError::SearchCountOverflow)?;
    }
    Ok(total)
}

mod tests;
