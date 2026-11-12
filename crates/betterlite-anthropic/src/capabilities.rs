pub mod budget;
pub mod effort;
pub mod effort_param;
pub mod reasoning;
pub mod speed;
pub mod supported_params;
mod tests;

use betterlite_config::sections::anthropic_capability_catalog::{
    HIGH_THINKING_BUDGET, LOW_THINKING_BUDGET, MAX_THINKING_BUDGET, MEDIUM_THINKING_BUDGET,
    MINIMAL_THINKING_BUDGET, XHIGH_THINKING_BUDGET,
};
use serde::{Deserialize, Serialize};

// LiteLLM's ANTHROPIC_MIN_THINKING_BUDGET_TOKENS, the floor a legacy minimal budget is raised to.
pub const MIN_THINKING_BUDGET_TOKENS: i64 = 1024;

// The six DEFAULT_REASONING_EFFORT_* budgets betterlite-config reads from the environment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThinkingBudgets {
    pub minimal: i64,
    pub low: i64,
    pub medium: i64,
    pub high: i64,
    pub xhigh: i64,
    pub max: i64,
}

impl Default for ThinkingBudgets {
    fn default() -> Self {
        Self {
            minimal: MINIMAL_THINKING_BUDGET,
            low: LOW_THINKING_BUDGET,
            medium: MEDIUM_THINKING_BUDGET,
            high: HIGH_THINKING_BUDGET,
            xhigh: XHIGH_THINKING_BUDGET,
            max: MAX_THINKING_BUDGET,
        }
    }
}
