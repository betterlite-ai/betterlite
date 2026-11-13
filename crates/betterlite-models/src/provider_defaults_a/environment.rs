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

// An empty env var counts as unset, as docs/compatibility.md says for every environment value.
pub(crate) fn first_set<E>(env: &E, names: &[&str]) -> Option<String>
where
    E: EnvironmentLookup,
{
    names
        .iter()
        .find_map(|name| env.lookup(name).filter(|value| !value.is_empty()))
}
