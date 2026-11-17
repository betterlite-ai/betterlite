use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const PROMPT_CACHING_CHECK: &str = "prompt_caching";
const BASE64_LIMIT_VARIABLE: &str = "MAX_BASE64_LENGTH_FOR_LOGGING";

// Router.add_optional_pre_call_checks builds PromptCachingDeploymentCheck when the list names prompt_caching
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    optional_pre_call_checks: Option<Vec<String>>,
}

// the check is off unless the config names it; the base64 limit is the logging placeholder threshold
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub prompt_caching: bool,
    pub maximum_base64_length: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let checks = self
            .router_settings
            .optional_pre_call_checks
            .unwrap_or_default();
        Ok(Settings {
            prompt_caching: checks.iter().any(|check| check == PROMPT_CACHING_CHECK),
            maximum_base64_length: lookup.lookup(BASE64_LIMIT_VARIABLE),
        })
    }
}
