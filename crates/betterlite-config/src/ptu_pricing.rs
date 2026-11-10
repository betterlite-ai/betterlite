use std::collections::BTreeMap;

use betterlite_models::ptu_pricing::PTU_COST_ATTRIBUTION_ENV_VAR as CORE_ENV_VAR;
use betterlite_secret_managers::secret_lookup::SecretLookup;

// The proxy's flag module only re-exports the core name, which betterlite spells as this value alias.
pub const PTU_COST_ATTRIBUTION_ENV_VAR: &str = CORE_ENV_VAR;

mod tests;

// LiteLLM returns the default false when the secret read raises, so a lookup error leaves attribution off.
#[must_use]
pub fn load_ptu_cost_flag(lookup: &SecretLookup, environment: &BTreeMap<String, String>) -> bool {
    matches!(
        lookup.get_secret_bool(environment, PTU_COST_ATTRIBUTION_ENV_VAR, Some(false)),
        Ok(Some(true))
    )
}
