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

// Settings fields in this order: minimal, low, medium, high, xhigh, max, bedrock_min.
const BUDGETS: [(&str, i64); 7] = [
    (
        "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET",
        MINIMAL_THINKING_BUDGET,
    ),
    (
        "DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET",
        LOW_THINKING_BUDGET,
    ),
    (
        "DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET",
        MEDIUM_THINKING_BUDGET,
    ),
    (
        "DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET",
        HIGH_THINKING_BUDGET,
    ),
    (
        "DEFAULT_REASONING_EFFORT_XHIGH_THINKING_BUDGET",
        XHIGH_THINKING_BUDGET,
    ),
    (
        "DEFAULT_REASONING_EFFORT_MAX_THINKING_BUDGET",
        MAX_THINKING_BUDGET,
    ),
    (
        "BEDROCK_MIN_THINKING_BUDGET_TOKENS",
        BEDROCK_MIN_THINKING_BUDGET,
    ),
];

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
        let mut values = [0_i64; 7];
        for (value, (name, default)) in values.iter_mut().zip(BUDGETS) {
            // An unset variable reads as empty text, and the shared parser treats empty as unset.
            let text = lookup.lookup(name).unwrap_or_default();
            *value = betterlite_env::number::<i64>(name, &text)?.unwrap_or(default);
        }
        let [minimal, low, medium, high, xhigh, max, bedrock_min] = values;
        Ok(Settings {
            minimal,
            low,
            medium,
            high,
            xhigh,
            max,
            bedrock_min,
        })
    }
}
