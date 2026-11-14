mod tests;

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const MAX_TOKENS_VARIABLE: &str = "BACKGROUND_HEALTH_CHECK_MAX_TOKENS";
const MAX_TOKENS_REASONING_VARIABLE: &str = "BACKGROUND_HEALTH_CHECK_MAX_TOKENS_REASONING";
const TIMEOUT_VARIABLE: &str = "HEALTH_CHECK_TIMEOUT_SECONDS";
const PROMPT_VARIABLE: &str = "DEFAULT_HEALTH_CHECK_PROMPT";
const DEFAULT_PROMPT: &str = "test from litellm";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const SKIP_DISABLED_PATH: &str = "general_settings.health_check_skip_disabled_background_models";
const MODEL_GROUPS_PATH: &str = "general_settings.background_health_check_model_groups";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "health_check_skip_disabled_background_models")]
    skip_disabled_models: Option<RawFlag>,
    #[serde(default, rename = "background_health_check_model_groups")]
    model_groups: Option<RawGroups>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawGroups {
    Groups(Vec<String>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub skip_disabled_models: bool,
    pub model_groups: Option<BTreeSet<String>>,
    pub max_tokens: Option<i64>,
    pub max_tokens_reasoning: Option<i64>,
    pub timeout: Duration,
    pub prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            skip_disabled_models: false,
            model_groups: None,
            max_tokens: None,
            max_tokens_reasoning: None,
            timeout: DEFAULT_TIMEOUT,
            prompt: DEFAULT_PROMPT.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let skip_disabled_models = match general.skip_disabled_models {
            None => false,
            Some(RawFlag::Boolean(value)) => value,
            Some(RawFlag::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: SKIP_DISABLED_PATH.to_owned(),
                    value: false,
                });
            }
        };
        let model_groups = match general.model_groups {
            None => None,
            Some(RawGroups::Groups(groups)) => Some(groups.into_iter().collect()),
            Some(RawGroups::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: MODEL_GROUPS_PATH.to_owned(),
                    value: false,
                });
            }
        };
        let configured_timeout = betterlite_env::number::<i64>(
            TIMEOUT_VARIABLE,
            &lookup.lookup(TIMEOUT_VARIABLE).unwrap_or_default(),
        )?;
        // asyncio.wait_for takes a negative timeout and times every probe out at once
        let timeout = configured_timeout.map_or(DEFAULT_TIMEOUT, |seconds| {
            Duration::from_secs(u64::try_from(seconds).unwrap_or_default())
        });
        let prompt = lookup
            .lookup(PROMPT_VARIABLE)
            .unwrap_or_else(|| DEFAULT_PROMPT.to_owned());
        Ok(Settings {
            skip_disabled_models,
            model_groups,
            // LiteLLM's import-time try/except turns a malformed token variable into unset
            max_tokens: token_limit(lookup, MAX_TOKENS_VARIABLE),
            max_tokens_reasoning: token_limit(lookup, MAX_TOKENS_REASONING_VARIABLE),
            timeout,
            prompt,
        })
    }
}

fn token_limit<L>(lookup: &L, variable: &str) -> Option<i64>
where
    L: EnvironmentLookup,
{
    betterlite_env::number::<i64>(variable, &lookup.lookup(variable).unwrap_or_default())
        .ok()
        .flatten()
}
