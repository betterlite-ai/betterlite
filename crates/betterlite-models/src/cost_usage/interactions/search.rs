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

/// Sums integer counts from mapping entries whose `type` is exactly `google_search`.
/// A non-mapping entry, or one whose type does not match, is skipped rather than
/// treated as an error: only the modality aggregation crashes on a bad shape.
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
