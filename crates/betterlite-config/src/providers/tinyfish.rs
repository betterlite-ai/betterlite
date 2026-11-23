use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::sections::websearch_request_hooks::Settings as WebsearchRequestHooksSettings;

const DEFAULT_API_BASE: &str = "https://api.search.tinyfish.ai";
const API_KEY_VAR: &str = "TINYFISH_API_KEY";
const API_BASE_VAR: &str = "TINYFISH_API_BASE";

// clippy's multiple_inherent_impl gives each provider's resolver a config type of its own
impl WebsearchRequestHooksSettings {
    // LiteLLM's TinyfishSearchConfig reads both values from the environment, so this stays a lookup over the loader
    #[must_use]
    pub fn tinyfish_search_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        (
            present(API_KEY_VAR).map(SecretString::from),
            present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
