use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::ModelDeployment;

const DEFAULT_API_BASE: &str = "https://sdk.nimbleway.com/v2";
const API_KEY_VAR: &str = "NIMBLE_API_KEY";
const API_BASE_VAR: &str = "NIMBLE_API_BASE";

// clippy's multiple_inherent_impl gives each provider's resolver a config type of its own
impl ModelDeployment {
    #[must_use]
    pub fn nimble_search_settings(
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
