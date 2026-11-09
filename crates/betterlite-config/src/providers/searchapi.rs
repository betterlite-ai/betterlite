use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::Config;

const DEFAULT_API_BASE: &str = "https://www.searchapi.io/api/v1/search";
const API_KEY_VAR: &str = "SEARCHAPI_API_KEY";
const API_BASE_VAR: &str = "SEARCHAPI_API_BASE";

impl Config {
    #[must_use]
    pub fn searchapi_search_settings(
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
