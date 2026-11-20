use std::collections::BTreeSet;

// LiteLLM keeps the first sighting of every name, so a repeated tag or group never adds a second counter
pub(crate) fn dedupe_names(names: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::with_capacity(names.len());
    for name in names {
        if seen.insert(name.as_str()) {
            deduped.push(name.clone());
        }
    }
    deduped
}
