use betterlite_env::EnvError;

use crate::loader::EnvironmentLookup;
use crate::types::LiteLlmSettings;

const MINIMAL_VARIABLE: &str = "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET";
const MINIMAL_FLASH_VARIABLE: &str =
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH";
const MINIMAL_PRO_VARIABLE: &str =
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO";
const MINIMAL_FLASH_LITE_VARIABLE: &str =
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE";
const LOW_VARIABLE: &str = "DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET";
const MEDIUM_VARIABLE: &str = "DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET";
const HIGH_VARIABLE: &str = "DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET";
const DISABLE_VARIABLE: &str = "DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET";

const MINIMAL_DEFAULT: i64 = 128;
const MINIMAL_FLASH_DEFAULT: i64 = 1;
const MINIMAL_PRO_DEFAULT: i64 = 128;
const MINIMAL_FLASH_LITE_DEFAULT: i64 = 512;
const LOW_DEFAULT: i64 = 1024;
const MEDIUM_DEFAULT: i64 = 2048;
const HIGH_DEFAULT: i64 = 4096;
const DISABLE_DEFAULT: i64 = 0;

// the budgets follow LiteLLM's constants order: minimal, minimal-flash, minimal-pro, minimal-flash-lite, low, medium, high, disable
pub(crate) type GeminiThinkingSettings = ([i64; 8], bool);

impl LiteLlmSettings {
    pub fn gemini_thinking_settings(
        configured_low: Option<bool>,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<GeminiThinkingSettings, EnvError> {
        let budgets = [
            env_number(lookup, MINIMAL_VARIABLE, MINIMAL_DEFAULT)?,
            env_number(lookup, MINIMAL_FLASH_VARIABLE, MINIMAL_FLASH_DEFAULT)?,
            env_number(lookup, MINIMAL_PRO_VARIABLE, MINIMAL_PRO_DEFAULT)?,
            env_number(
                lookup,
                MINIMAL_FLASH_LITE_VARIABLE,
                MINIMAL_FLASH_LITE_DEFAULT,
            )?,
            env_number(lookup, LOW_VARIABLE, LOW_DEFAULT)?,
            env_number(lookup, MEDIUM_VARIABLE, MEDIUM_DEFAULT)?,
            env_number(lookup, HIGH_VARIABLE, HIGH_DEFAULT)?,
            env_number(lookup, DISABLE_VARIABLE, DISABLE_DEFAULT)?,
        ];
        Ok((budgets, configured_low.unwrap_or(false)))
    }
}

fn env_number(lookup: &dyn EnvironmentLookup, name: &str, default: i64) -> Result<i64, EnvError> {
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::number::<i64>(name, &text)?.unwrap_or(default))
}

mod tests;
