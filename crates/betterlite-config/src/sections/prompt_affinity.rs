use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const PROMPT_CACHING_CHECK: &str = "prompt_caching";
const BASE64_LIMIT_VARIABLE: &str = "MAX_BASE64_LENGTH_FOR_LOGGING";
// litellm.constants.MAX_BASE64_LENGTH_FOR_LOGGING reads the variable once, defaulting to 64
const DEFAULT_BASE64_LIMIT: i64 = 64;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub prompt_caching: bool,
    pub maximum_base64_length: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            prompt_caching: false,
            maximum_base64_length: DEFAULT_BASE64_LIMIT,
        }
    }
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
        let configured = match lookup.lookup(BASE64_LIMIT_VARIABLE) {
            Some(text) => betterlite_env::number::<i64>(BASE64_LIMIT_VARIABLE, &text)?,
            None => None,
        };
        Ok(Settings {
            prompt_caching: checks.iter().any(|check| check == PROMPT_CACHING_CHECK),
            maximum_base64_length: configured.unwrap_or(DEFAULT_BASE64_LIMIT),
        })
    }
}
