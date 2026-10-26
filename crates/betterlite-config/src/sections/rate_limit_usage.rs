use betterlite_rate_limit::usage_resolution::TokenRateLimitType;
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
    pub token_rate_limit_type: TokenRateLimitType,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            token_rate_limit_type: TokenRateLimitType::Total,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let token_rate_limit_type =
            TokenRateLimitType::from_config(self.general_settings.token_rate_limit_type.as_deref());
        Ok(Settings {
            token_rate_limit_type,
        })
    }
}

mod tests;
