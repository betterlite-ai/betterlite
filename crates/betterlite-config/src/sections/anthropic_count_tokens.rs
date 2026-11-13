use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const BETA_VERSION_VARIABLE: &str = "ANTHROPIC_TOKEN_COUNTING_BETA_VERSION";
pub const DEFAULT_BETA_VERSION: &str = "token-counting-2024-11-01";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // LiteLLM reads the beta only from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub beta_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            beta_version: DEFAULT_BETA_VERSION.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            beta_version: beta_version(lookup),
        })
    }
}

// LiteLLM reads the variable with os.getenv; the project's compatibility rules make an empty value unset
fn beta_version<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    match lookup.lookup(BETA_VERSION_VARIABLE) {
        Some(value) if !value.is_empty() => value,
        Some(_) | None => DEFAULT_BETA_VERSION.to_owned(),
    }
}
