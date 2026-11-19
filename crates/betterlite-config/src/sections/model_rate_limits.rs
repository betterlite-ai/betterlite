use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ENFORCE_MODEL_RATE_LIMITS: &str = "enforce_model_rate_limits";

// Router.add_optional_pre_call_checks builds ModelRateLimitingCheck when the list names enforce_model_rate_limits
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    optional_pre_call_checks: Option<Vec<String>>,
}

// The check is off unless the config names it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub enforce_model_rate_limits: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let checks = self
            .router_settings
            .optional_pre_call_checks
            .unwrap_or_default();
        Ok(Settings {
            enforce_model_rate_limits: checks
                .iter()
                .any(|check| check == ENFORCE_MODEL_RATE_LIMITS),
        })
    }
}
