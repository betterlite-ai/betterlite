use std::collections::{BTreeMap, BTreeSet};

use super::settings_rules::JsonValue;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionChanges {
    pub changed: BTreeMap<String, JsonValue>,
    pub removed: BTreeSet<String>,
}

#[must_use]
pub fn changed_section_keys(
    baseline: &BTreeMap<String, JsonValue>,
    new: &BTreeMap<String, JsonValue>,
) -> SectionChanges {
    let mut changes = SectionChanges::default();
    for (key, value) in new {
        if baseline.get(key) != Some(value) {
            changes.changed.insert(key.clone(), value.clone());
        }
    }
    for key in baseline.keys() {
        if !new.contains_key(key) {
            changes.removed.insert(key.clone());
        }
    }
    changes
}
