use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

pub const LOW_THINKING_BUDGET: i64 = 1024;
pub const MEDIUM_THINKING_BUDGET: i64 = 2048;
pub const HIGH_THINKING_BUDGET: i64 = 4096;

#[must_use]
pub const fn effort_from_thinking_budget(budget_tokens: i64) -> ReasoningEffort {
    if budget_tokens >= HIGH_THINKING_BUDGET {
        return ReasoningEffort::High;
    }
    if budget_tokens >= MEDIUM_THINKING_BUDGET {
        return ReasoningEffort::Medium;
    }
    if budget_tokens >= LOW_THINKING_BUDGET {
        return ReasoningEffort::Low;
    }
    ReasoningEffort::Minimal
}

mod tests;
