use indexmap::IndexMap;

use crate::models::finding::{Evidence, MAX_EVIDENCE};

mod tests;

#[must_use]
pub fn merge_evidence(previous: &[Evidence], draft: &[Evidence]) -> Vec<Evidence> {
    // a repeated key keeps its first position and takes the newest value, as a Python dict does
    let unique: IndexMap<(&str, &str, &str), &Evidence> = previous
        .iter()
        .chain(draft)
        .map(|evidence| {
            let key = (
                evidence.execution_id.as_str(),
                evidence.span_id.as_str(),
                evidence.quote.as_str(),
            );
            (key, evidence)
        })
        .collect();
    let excess = unique.len().saturating_sub(MAX_EVIDENCE);
    unique.into_values().skip(excess).cloned().collect()
}
