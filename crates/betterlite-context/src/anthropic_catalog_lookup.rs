use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use candidates::CandidatePatterns;
use error::CatalogLookupError;

pub mod candidates;
pub mod error;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Capability {
    Flag(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CatalogRecord {
    Fields(BTreeMap<String, Capability>),
    Malformed(IgnoredAny),
}

pub type RawCatalog = BTreeMap<String, CatalogRecord>;

pub fn exact_capability(
    catalog: &RawCatalog,
    model: &str,
    key: &str,
) -> Result<Option<bool>, CatalogLookupError> {
    match catalog.get(model) {
        None => Ok(None),
        Some(CatalogRecord::Fields(fields)) => Ok(match fields.get(key) {
            Some(Capability::Flag(flag)) => Some(*flag),
            Some(Capability::Other(_)) | None => None,
        }),
        Some(CatalogRecord::Malformed(_)) => Err(CatalogLookupError::MalformedEntry {
            model: model.to_owned(),
        }),
    }
}

// Every candidate is tried in the runtime map before any in the packaged one; an error ends the search.
#[must_use]
pub fn model_capability(
    patterns: &CandidatePatterns,
    runtime: &RawCatalog,
    packaged: &RawCatalog,
    model: &str,
    key: &str,
) -> Option<bool> {
    let candidates = patterns.candidates(model);
    for catalog in [runtime, packaged] {
        for candidate in &candidates {
            let Ok(flag) = exact_capability(catalog, candidate, key) else {
                return None;
            };
            if flag.is_some() {
                return flag;
            }
        }
    }
    None
}

pub fn supports_fast_mode(
    runtime: &RawCatalog,
    model: &str,
    provider: &str,
) -> Result<bool, CatalogLookupError> {
    Ok(provider == "anthropic"
        && exact_capability(runtime, model, "supports_fast_mode")? == Some(true))
}
