use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// this section reads no YAML key; LiteLLM reads all three names from the process environment
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub region: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: lookup.lookup("BEDROCK_MANTLE_API_BASE"),
            api_key: lookup
                .lookup("BEDROCK_MANTLE_API_KEY")
                .map(SecretString::from),
            region: lookup.lookup("BEDROCK_MANTLE_REGION"),
        })
    }
}

mod tests;
