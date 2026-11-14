use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    enable_pre_call_checks: Option<bool>,
}

// enable_pre_call_checks is the switch over the context-window and supported-parameter filters; it defaults off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub enable_pre_call_checks: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            enable_pre_call_checks: self.router_settings.enable_pre_call_checks.unwrap_or(false),
        })
    }
}
