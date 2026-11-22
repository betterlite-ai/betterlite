use crate::loader::EnvironmentLookup;
use secrecy::SecretString;
const DEFAULT_API_BASE: &str = "https://api.search.tinyfish.ai";
const API_KEY_VARIABLE: &str = "TINYFISH_API_KEY";
const API_BASE_VARIABLE: &str = "TINYFISH_API_BASE";
// LiteLLM's TinyfishSearchConfig reads both values from the environment, so this stays a free function over the lookup
#[must_use]
pub fn tinyfish_search_settings(lookup: &dyn EnvironmentLookup) -> (Option<SecretString>, String) {
    let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
    (
        present(API_KEY_VARIABLE).map(SecretString::from),
        present(API_BASE_VARIABLE).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    )
}
mod tests;
