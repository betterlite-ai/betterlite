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
    health_check_skip_disabled_background_models: Option<RawFlag>,
    background_health_check_model_groups: Option<RawGroups>,
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
    pub skip_disabled_background_models: bool,
    pub background_model_groups: Option<BTreeSet<String>>,
    pub background_max_tokens: Option<i64>,
    pub background_max_tokens_reasoning: Option<i64>,
    pub timeout: Duration,
    pub prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            skip_disabled_background_models: false,
            background_model_groups: None,
            background_max_tokens: None,
            background_max_tokens_reasoning: None,
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
        let skip_disabled_background_models =
            match general.health_check_skip_disabled_background_models {
                None => false,
                Some(RawFlag::Boolean(value)) => value,
                Some(RawFlag::Other(_)) => {
                    return Err(ConfigError::InvalidScalar {
                        path: SKIP_DISABLED_PATH.to_owned(),
                        value: false,
                    });
                }
            };
        let background_model_groups = match general.background_health_check_model_groups {
            None => None,
            Some(RawGroups::Groups(groups)) => Some(groups.into_iter().collect()),
            Some(RawGroups::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: MODEL_GROUPS_PATH.to_owned(),
                    value: false,
                });
            }
        };
        let timeout = betterlite_env::number::<u64>(
            TIMEOUT_VARIABLE,
            &lookup.lookup(TIMEOUT_VARIABLE).unwrap_or_default(),
        )?
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs);
        let prompt = lookup
            .lookup(PROMPT_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_PROMPT.to_owned());
        Ok(Settings {
            skip_disabled_background_models,
            background_model_groups,
            // LiteLLM's import-time try/except turns a malformed token variable into unset
            background_max_tokens: token_limit(lookup, MAX_TOKENS_VARIABLE),
            background_max_tokens_reasoning: token_limit(lookup, MAX_TOKENS_REASONING_VARIABLE),
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
