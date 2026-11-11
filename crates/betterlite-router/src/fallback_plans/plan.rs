use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};

use crate::fallback_plans::attempts::{AttemptedTargets, has_unattempted_targets};
use crate::fallback_plans::chains::{ProviderNames, resolve_for_lookup_groups};

// the fallback chains and depth bound a router holds between requests
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackPlans {
    pub fallbacks: Vec<FallbackChainEntry>,
    pub context_window_fallbacks: Vec<FallbackChainEntry>,
    pub content_policy_fallbacks: Vec<FallbackChainEntry>,
    pub max_fallbacks: i64,
}

// The values one request overrides the router's plans with; a present but empty chain disables that fallback
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestOverrides<'request> {
    pub fallbacks: Option<&'request [FallbackChainEntry]>,
    pub context_window_fallbacks: Option<&'request [FallbackChainEntry]>,
    pub content_policy_fallbacks: Option<&'request [FallbackChainEntry]>,
    pub depth: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FallbackBudget {
    pub depth: i64,
    pub max_fallbacks: i64,
}

// The chains and the attempt budget one request runs with
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedChains<'plan> {
    pub ordinary: &'plan [FallbackChainEntry],
    pub context_window: &'plan [FallbackChainEntry],
    pub content_policy: &'plan [FallbackChainEntry],
    pub budget: FallbackBudget,
}

// The targets one lookup name resolves to, and whether any of them is still unattempted
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChain<'plan> {
    pub targets: Vec<&'plan FallbackTarget>,
    pub has_unattempted: bool,
}

// LiteLLM stops before the fallback attempt at or past max_fallbacks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthOutcome {
    Continue { next_depth: i64 },
    Exhausted,
}

impl FallbackPlans {
    #[must_use]
    pub fn chains<'plan>(&'plan self, request: &RequestOverrides<'plan>) -> SelectedChains<'plan> {
        SelectedChains {
            ordinary: request.fallbacks.unwrap_or(&self.fallbacks),
            context_window: request
                .context_window_fallbacks
                .unwrap_or(&self.context_window_fallbacks),
            content_policy: request
                .content_policy_fallbacks
                .unwrap_or(&self.content_policy_fallbacks),
            budget: FallbackBudget {
                depth: request.depth,
                max_fallbacks: self.max_fallbacks,
            },
        }
    }
}

#[must_use]
pub fn resolve_chain<'plan>(
    chain: &'plan [FallbackChainEntry],
    lookup_groups: &[&str],
    names: &ProviderNames<'_>,
    attempted: &AttemptedTargets,
) -> Option<ResolvedChain<'plan>> {
    let found = resolve_for_lookup_groups(chain, lookup_groups, names)?;
    let has_unattempted = has_unattempted_targets(Some(&found.targets), attempted);
    Some(ResolvedChain {
        targets: found.targets,
        has_unattempted,
    })
}

#[must_use]
pub const fn advance_depth(budget: FallbackBudget) -> DepthOutcome {
    if budget.depth >= budget.max_fallbacks {
        return DepthOutcome::Exhausted;
    }
    DepthOutcome::Continue {
        next_depth: budget.depth.saturating_add(1),
    }
}
