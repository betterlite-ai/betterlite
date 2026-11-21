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
    #[serde(default)]
    fail_closed_budget_enforcement: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    // LiteLLM's `is True`: only an explicit true rejects a budget check whose spend nothing verified
    pub fail_closed_budget_enforcement: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            fail_closed_budget_enforcement: self
                .general_settings
                .fail_closed_budget_enforcement
                .unwrap_or_default(),
        })
    }
}

mod tests;
