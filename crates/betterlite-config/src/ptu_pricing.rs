use std::collections::BTreeMap;

use betterlite_models::ptu_pricing::PTU_COST_ATTRIBUTION_ENV_VAR as CORE_ENV_VAR;
use betterlite_secret_managers::secret_lookup::text_to_bool;

// The proxy's flag module only re-exports the core name, which betterlite spells as this value alias.
pub const PTU_COST_ATTRIBUTION_ENV_VAR: &str = CORE_ENV_VAR;

mod tests;

// get_secret_bool's environment branch is this same text rule, so the flag ends up true only for "true".
#[must_use]
pub fn load_ptu_cost_flag(environment: &BTreeMap<String, String>) -> bool {
    environment
        .get(PTU_COST_ATTRIBUTION_ENV_VAR)
        .and_then(|text| text_to_bool(text))
        == Some(true)
}
