use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThinkingBudgetThresholds {
    pub low: i64,
    pub medium: i64,
    pub high: i64,
}

// The config boundary resolves the DEFAULT_REASONING_EFFORT_LOW/MEDIUM/HIGH_THINKING_BUDGET env vars into these defaults.
pub const LOW_THINKING_BUDGET: i64 = 1024;
pub const MEDIUM_THINKING_BUDGET: i64 = 2048;
pub const HIGH_THINKING_BUDGET: i64 = 4096;

impl Default for ThinkingBudgetThresholds {
    fn default() -> Self {
        Self {
            low: LOW_THINKING_BUDGET,
            medium: MEDIUM_THINKING_BUDGET,
            high: HIGH_THINKING_BUDGET,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

#[must_use]
pub const fn effort_from_thinking_budget(
    budget_tokens: i64,
    thresholds: ThinkingBudgetThresholds,
) -> ReasoningEffort {
    if budget_tokens >= thresholds.high {
        return ReasoningEffort::High;
    }
    if budget_tokens >= thresholds.medium {
        return ReasoningEffort::Medium;
    }
    if budget_tokens >= thresholds.low {
        return ReasoningEffort::Low;
    }
    ReasoningEffort::Minimal
}

mod tests;
