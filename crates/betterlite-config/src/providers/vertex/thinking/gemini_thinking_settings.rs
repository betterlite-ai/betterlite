use betterlite_env::EnvError;

use crate::loader::EnvironmentLookup;
use crate::types::LiteLlmSettings;

const VARIABLES: [&str; 8] = [
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET",
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH",
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO",
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE",
    "DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET",
    "DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET",
    "DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET",
    "DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET",
];
const DEFAULTS: [i64; 8] = [128, 1, 128, 512, 1024, 2048, 4096, 0];

// the budgets follow LiteLLM's constants order: minimal, minimal-flash, minimal-pro, minimal-flash-lite, low, medium, high, disable
pub(crate) type GeminiThinkingSettings = ([i64; 8], bool);

impl LiteLlmSettings {
    pub fn gemini_thinking_settings(
        configured_low: Option<bool>,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<GeminiThinkingSettings, EnvError> {
        let mut budgets = DEFAULTS;
        for (budget, name) in budgets.iter_mut().zip(VARIABLES) {
            *budget = env_number(lookup, name, *budget)?;
        }
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
