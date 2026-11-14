use betterlite_secret_managers::secret_lookup::text_to_bool;
use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::LiteLlmParams;

const DEFAULT_API_BASE: &str = "https://api.elevenlabs.io";
const API_KEY_VAR: &str = "ELEVENLABS_API_KEY";
const API_BASE_VAR: &str = "ELEVENLABS_API_BASE";

impl LiteLlmParams {
    // get_secret_str reads true or false as unset; the key comes last in an `or` chain, so an empty one stays as given
    #[must_use]
    pub fn elevenlabs_speech_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let api_base = lookup
            .lookup(API_BASE_VAR)
            .filter(|value| !value.is_empty() && text_to_bool(value).is_none())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let api_key = lookup
            .lookup(API_KEY_VAR)
            .filter(|value| text_to_bool(value).is_none())
            .map(SecretString::from);
        (api_key, api_base)
    }
}

mod tests;
