use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::settings_rules::JsonValue;

mod tests;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SectionChanges {
    pub changed: BTreeMap<String, JsonValue>,
    pub removed: BTreeSet<String>,
}

impl fmt::Debug for SectionChanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SectionChanges")
            .field("changed_keys", &self.changed.keys().collect::<Vec<_>>())
            .field("removed_keys", &self.removed)
            .finish()
    }
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
