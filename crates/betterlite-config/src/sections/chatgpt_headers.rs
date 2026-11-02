use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::chatgpt::headers as environment;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub originator_override: Option<String>,
    pub user_agent_override: Option<String>,
    pub user_agent_suffix: Option<String>,
    pub default_instructions_override: Option<String>,
    pub terminal: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(environment::resolve(lookup))
    }
}

mod tests;
