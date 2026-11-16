use betterlite_secret_managers::secret_lookup::text_to_bool;
use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::RoutingStrategy;

const DEFAULT_API_BASE: &str = "https://external.api.recraft.ai";
const API_KEY_VARIABLE: &str = "RECRAFT_API_KEY";
const API_BASE_VARIABLE: &str = "RECRAFT_API_BASE";

// clippy's multiple_inherent_impl gives each provider's resolver a config type of its own; stability holds ProxyTrust
impl RoutingStrategy {
    // get_secret_str reads a true or false value as unset, so both variables need the same filter
    #[must_use]
    pub fn recraft_image_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let present = |name: &str| {
            lookup
                .lookup(name)
                .filter(|value| !value.is_empty() && text_to_bool(value).is_none())
        };
        (
            present(API_KEY_VARIABLE).map(SecretString::from),
            present(API_BASE_VARIABLE).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
