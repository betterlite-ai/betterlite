use std::collections::BTreeSet;

use betterlite_compression::protected_indices::get_protected_indices as compression_indices;
use betterlite_prompts::arguments::json::Json;

use crate::typesafe::{GetProtectedIndices, GroupToolExchanges};

// Compression reads serde_json values, so each message converts one for one and keeps its index.
#[must_use]
pub fn compression_protected_indices(messages: &[Json]) -> Vec<usize> {
    let values: Vec<_> = messages
        .iter()
        .map(|message| serde_json::to_value(message).unwrap_or_default())
        .collect();
    compression_indices(&values)
}

#[must_use]
pub fn protected_indices(
    messages: &[Json],
    get_protected_indices: GetProtectedIndices,
    group_tool_exchanges: GroupToolExchanges,
) -> BTreeSet<usize> {
    let shared: BTreeSet<usize> = get_protected_indices(messages).into_iter().collect();
    let mut protected = shared.clone();
    for group in group_tool_exchanges(messages) {
        if group.iter().any(|member| shared.contains(member)) {
            protected.extend(group);
        }
    }
    protected
}

#[path = "typesafe_protected_indices/tests.rs"]
mod tests;
