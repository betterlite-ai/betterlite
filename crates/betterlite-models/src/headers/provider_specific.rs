use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSpecificHeader {
    #[serde(default)]
    pub custom_llm_provider: String,
    #[serde(default)]
    pub extra_headers: IndexMap<String, String>,
}

// Each entry carries its own comma-separated provider scope; an entry outside that scope contributes nothing, and a later entry wins on a repeated header name.
#[must_use]
pub fn provider_specific_headers(
    entries: Option<&[ProviderSpecificHeader]>,
    custom_llm_provider: Option<&str>,
) -> IndexMap<String, String> {
    let (Some(scoped_entries), Some(provider)) = (entries, custom_llm_provider) else {
        return IndexMap::new();
    };
    let mut matched = IndexMap::new();
    for entry in scoped_entries {
        let in_scope = entry
            .custom_llm_provider
            .split(',')
            .any(|scoped| scoped.trim() == provider);
        if !in_scope {
            continue;
        }
        for (name, value) in &entry.extra_headers {
            matched.insert(name.clone(), value.clone());
        }
    }
    matched
}

mod tests;
