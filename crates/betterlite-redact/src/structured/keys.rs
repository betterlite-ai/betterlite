use super::SensitiveDataMasker;
use std::collections::BTreeSet;

pub(super) fn sensitive(masker: &SensitiveDataMasker, key: &str) -> bool {
    let normalized_key = key.to_lowercase().replace('-', "_");
    let segments = normalized_key.split('_').collect::<BTreeSet<_>>();
    if masker
        .non_sensitive_overrides
        .iter()
        .any(|item| segments.contains(item.as_str()))
    {
        return false;
    }
    masker
        .sensitive_patterns
        .iter()
        .any(|item| segments.contains(item.as_str()))
}

mod tests;
