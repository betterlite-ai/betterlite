use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

const FINGERPRINT_PREFIXES: [&str; 4] = ["OPENAI", "ANTHROPIC", "AZURE", "AWS"];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvironmentSnapshot {
    pub names: BTreeSet<String>,
    pub fingerprint: u64,
}

impl From<&BTreeMap<String, String>> for EnvironmentSnapshot {
    fn from(entries: &BTreeMap<String, String>) -> Self {
        let mut hasher = DefaultHasher::new();
        for (name, value) in entries {
            if FINGERPRINT_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                name.hash(&mut hasher);
                value.hash(&mut hasher);
            }
        }
        Self {
            names: entries.keys().cloned().collect(),
            fingerprint: hasher.finish(),
        }
    }
}
