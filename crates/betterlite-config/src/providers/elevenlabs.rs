use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::types::Config;

const DEFAULT_API_BASE: &str = "https://api.elevenlabs.io";
const API_KEY_VAR: &str = "ELEVENLABS_API_KEY";
const API_BASE_VAR: &str = "ELEVENLABS_API_BASE";

impl Config {
    // LiteLLM reads the key last in an `or` chain and accepts an empty one there, so it stays as given
    #[must_use]
    pub fn elevenlabs_speech_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let api_base = lookup
            .lookup(API_BASE_VAR)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        (lookup.lookup(API_KEY_VAR).map(SecretString::from), api_base)
    }
}

mod tests;
