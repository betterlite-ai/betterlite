use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const OUTPUT: &str = "output";
pub const INPUT: &str = "input";
pub const TOTAL: &str = "total";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenRateLimitType {
    Output,
    Input,
    #[default]
    Total,
}

impl TokenRateLimitType {
    #[must_use]
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some(OUTPUT) => Self::Output,
            Some(INPUT) => Self::Input,
            _ => Self::Total,
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => OUTPUT,
            Self::Input => INPUT,
            Self::Total => TOTAL,
        }
    }
}

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
