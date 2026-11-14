use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::ModelInfo;

const DEFAULT_API_BASE: &str = "https://api.parallel.ai";
const API_KEY_VARS: [&str; 2] = ["PARALLEL_AI_API_KEY", "PARALLEL_API_KEY"];
const API_BASE_VAR: &str = "PARALLEL_AI_API_BASE";

// clippy's multiple_inherent_impl gives each provider's resolver a config type of its own
impl ModelInfo {
    #[must_use]
    pub fn parallel_ai_search_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let api_key = API_KEY_VARS
            .iter()
            .find_map(|name| present(name))
            .map(SecretString::from);
        (
            api_key,
            present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
