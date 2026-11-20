use betterlite_secret_managers::secret_lookup::text_to_bool;
use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::provider_budget_view::ProviderBudgetConfig;

const DEFAULT_API_BASE: &str = "https://api.topazlabs.com";
const API_KEY_VARIABLE: &str = "TOPAZ_API_KEY";
const API_BASE_VARIABLE: &str = "TOPAZ_API_BASE";

// clippy's multiple_inherent_impl gives each provider's resolver a config type of its own; recraft holds RoutingStrategy
impl ProviderBudgetConfig {
    // get_secret_str reads an empty, true or false variable as unset, so both variables need the same filter
    #[must_use]
    pub fn topaz_image_settings(lookup: &dyn EnvironmentLookup) -> (Option<SecretString>, String) {
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
