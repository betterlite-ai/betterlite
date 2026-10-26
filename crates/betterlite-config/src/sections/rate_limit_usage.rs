use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    token_rate_limit_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub token_rate_limit_type: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            token_rate_limit_type: "total".to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let token_rate_limit_type = self
            .general_settings
            .token_rate_limit_type
            .unwrap_or_else(|| "total".to_owned());
        Ok(Settings {
            token_rate_limit_type,
        })
    }
}

mod tests;
