use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_MODEL_COST_MAP_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
pub const LOCAL_MODEL_COST_MAP_ENV: &str = "LITELLM_LOCAL_MODEL_COST_MAP";
pub const MODEL_COST_MAP_URL_ENV: &str = "LITELLM_MODEL_COST_MAP_URL";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // The operator configures this section through environment variables only, so every YAML key is ignored one by one
    #[serde(skip)]
    pub environment_only: (),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Settings {
    pub local: bool,
    pub url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            local: false,
            url: DEFAULT_MODEL_COST_MAP_URL.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let local_value = lookup.lookup(LOCAL_MODEL_COST_MAP_ENV).unwrap_or_default();
        let local = betterlite_env::flag(LOCAL_MODEL_COST_MAP_ENV, &local_value)?.unwrap_or(false);
        let url = match lookup.lookup(MODEL_COST_MAP_URL_ENV) {
            Some(configured) if !configured.is_empty() => configured,
            _ => DEFAULT_MODEL_COST_MAP_URL.to_owned(),
        };
        Ok(Settings { local, url })
    }
}

mod tests;
