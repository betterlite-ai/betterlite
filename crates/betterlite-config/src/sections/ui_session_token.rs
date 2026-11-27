mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const DURATION_ENV: &str = "LITELLM_UI_SESSION_DURATION";
const DEFAULT_DURATION: &str = "24h";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub duration: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            duration: DEFAULT_DURATION.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // os.getenv returns an empty value rather than the default, and LiteLLM parses it only when a token is minted
        let duration = lookup
            .lookup(DURATION_ENV)
            .unwrap_or_else(|| DEFAULT_DURATION.to_owned());
        Ok(Settings { duration })
    }
}
