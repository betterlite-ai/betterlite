#[path = "guardrail_modes/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub strict: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { strict: true }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM's str_to_bool reads only true and false, so any other text stays strict
        let strict = lookup
            .lookup("LITELLM_STRICT_GUARDRAIL_MODES")
            .is_none_or(|text| text.trim().to_lowercase() != "false");
        Ok(Settings { strict })
    }
}
