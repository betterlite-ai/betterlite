mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const MINIMAL_THINKING_BUDGET: i64 = 128;
pub const LOW_THINKING_BUDGET: i64 = 1024;
pub const MEDIUM_THINKING_BUDGET: i64 = 2048;
pub const HIGH_THINKING_BUDGET: i64 = 4096;
pub const XHIGH_THINKING_BUDGET: i64 = 8192;
pub const MAX_THINKING_BUDGET: i64 = 0x4000;
pub const BEDROCK_MIN_THINKING_BUDGET: i64 = 1024;

const MINIMAL_ENV: &str = "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET";
const LOW_ENV: &str = "DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET";
const MEDIUM_ENV: &str = "DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET";
const HIGH_ENV: &str = "DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET";
const XHIGH_ENV: &str = "DEFAULT_REASONING_EFFORT_XHIGH_THINKING_BUDGET";
const MAX_ENV: &str = "DEFAULT_REASONING_EFFORT_MAX_THINKING_BUDGET";
const BEDROCK_MIN_ENV: &str = "BEDROCK_MIN_THINKING_BUDGET_TOKENS";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // LiteLLM reads every budget only from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub minimal: i64,
    pub low: i64,
    pub medium: i64,
    pub high: i64,
    pub xhigh: i64,
    pub max: i64,
    pub bedrock_min: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            minimal: MINIMAL_THINKING_BUDGET,
            low: LOW_THINKING_BUDGET,
            medium: MEDIUM_THINKING_BUDGET,
            high: HIGH_THINKING_BUDGET,
            xhigh: XHIGH_THINKING_BUDGET,
            max: MAX_THINKING_BUDGET,
            bedrock_min: BEDROCK_MIN_THINKING_BUDGET,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            minimal: integer(lookup, MINIMAL_ENV, MINIMAL_THINKING_BUDGET)?,
            low: integer(lookup, LOW_ENV, LOW_THINKING_BUDGET)?,
            medium: integer(lookup, MEDIUM_ENV, MEDIUM_THINKING_BUDGET)?,
            high: integer(lookup, HIGH_ENV, HIGH_THINKING_BUDGET)?,
            xhigh: integer(lookup, XHIGH_ENV, XHIGH_THINKING_BUDGET)?,
            max: integer(lookup, MAX_ENV, MAX_THINKING_BUDGET)?,
            bedrock_min: integer(lookup, BEDROCK_MIN_ENV, BEDROCK_MIN_THINKING_BUDGET)?,
        })
    }
}

// int(os.getenv(...)): an empty value counts as unset, and a non-integer fails the load.
fn integer<L>(lookup: &L, name: &str, default: i64) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    match lookup.lookup(name) {
        Some(value) if !value.trim().is_empty() => {
            value
                .trim()
                .parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: name.to_owned(),
                    source,
                })
        }
        Some(_) | None => Ok(default),
    }
}
