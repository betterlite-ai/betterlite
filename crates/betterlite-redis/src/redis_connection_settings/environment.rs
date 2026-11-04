mod tests;

use std::collections::BTreeMap;

// Caller-supplied environment values, so this module never reads std::env itself.
pub trait EnvironmentLookup {
    fn lookup(&self, name: &str) -> Option<String>;
}

impl EnvironmentLookup for BTreeMap<String, String> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}
