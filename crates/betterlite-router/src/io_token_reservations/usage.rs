use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTokens {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
}

impl UsageTokens {
    // cached prompt-read tokens are excluded from ITPM accounting
    #[must_use]
    pub const fn billable_input(&self) -> i64 {
        match self.prompt_tokens.saturating_sub(self.cached_tokens) {
            billable if billable > 0 => billable,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileUsage {
    pub billable_input: i64,
    pub completion_tokens: i64,
    pub usage_resolved: bool,
}

// None stands for the pinned _usage_is_present false, which a usage carrying only total_tokens also is
#[must_use]
pub fn resolve_reconcile_usage(
    usage: Option<UsageTokens>,
    standard_logging: Option<UsageTokens>,
) -> ReconcileUsage {
    if let Some(resolved_usage) = usage {
        return ReconcileUsage {
            billable_input: resolved_usage.billable_input(),
            completion_tokens: resolved_usage.completion_tokens,
            usage_resolved: true,
        };
    }
    if let Some(stored) = standard_logging
        && (stored.prompt_tokens > 0 || stored.completion_tokens > 0)
    {
        return ReconcileUsage {
            billable_input: stored.billable_input(),
            completion_tokens: stored.completion_tokens,
            usage_resolved: true,
        };
    }
    ReconcileUsage::default()
}
