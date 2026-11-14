use betterlite_secret_managers::secret_lookup::text_to_bool;
use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::ModelDeployment;

const DEFAULT_API_BASE: &str = "https://api.soniox.com";
const API_KEY_VAR: &str = "SONIOX_API_KEY";
const API_BASE_VAR: &str = "SONIOX_API_BASE";

// the workspace denies a second inherent impl per type, and elevenlabs holds LiteLlmParams'
impl ModelDeployment {
    // get_secret_str reads true and false as unset, so both variables need the same filter
    #[must_use]
    pub fn soniox_record_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let api_base = lookup
            .lookup(API_BASE_VAR)
            .filter(|value| !value.is_empty() && text_to_bool(value).is_none())
            .map_or_else(
                || DEFAULT_API_BASE.to_owned(),
                |value| value.trim_end_matches('/').to_owned(),
            );
        let api_key = lookup
            .lookup(API_KEY_VAR)
            .filter(|value| text_to_bool(value).is_none())
            .map(SecretString::from);
        (api_key, api_base)
    }
}

mod tests;
