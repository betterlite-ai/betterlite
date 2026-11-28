use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const DISABLE_VARIABLE: &str = "LITELLM_DISABLE_BUG_REPORT_LINK";

// LiteLLM reads the flag from the environment alone, so this section deserializes no YAML key
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub disabled: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = lookup.lookup(DISABLE_VARIABLE).unwrap_or_default();
        let disabled = betterlite_env::flag(DISABLE_VARIABLE, &text)?.unwrap_or(false);
        Ok(Settings { disabled })
    }
}
