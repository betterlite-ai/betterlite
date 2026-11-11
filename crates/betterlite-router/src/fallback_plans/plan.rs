use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_config::sections::settings::JsonValue;

use crate::fallback_plans::attempts::{AttemptedTargets, has_unattempted_targets};
use crate::fallback_plans::chains::{ProviderNames, resolve_for_lookup_groups};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestOverrides<'request> {
    pub fallbacks: Option<&'request [FallbackChainEntry]>,
    pub context_window_fallbacks: Option<&'request [FallbackChainEntry]>,
    pub content_policy_fallbacks: Option<&'request [FallbackChainEntry]>,
    // the two retry controls ride along because a hop must not fall back to the router-level retry settings
    pub num_retries: Option<i64>,
    pub model_group_retry_policy: Option<&'request JsonValue>,
    pub depth: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FallbackBudget {
    pub depth: i64,
    pub max_fallbacks: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedChains<'plan> {
    pub ordinary: &'plan [FallbackChainEntry],
    pub context_window: &'plan [FallbackChainEntry],
    pub content_policy: &'plan [FallbackChainEntry],
    pub budget: FallbackBudget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChain<'plan> {
    pub targets: Vec<&'plan FallbackTarget>,
    pub has_unattempted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthOutcome {
    Continue { next_depth: i64 },
    Exhausted,
}

// A request-local chain replaces the configured one, and a present but empty chain keeps it out of the walk
#[must_use]
pub fn select_chains<'plan>(
    settings: &'plan Settings,
    request: &RequestOverrides<'plan>,
) -> SelectedChains<'plan> {
    SelectedChains {
        ordinary: request.fallbacks.unwrap_or(&settings.fallbacks),
        context_window: request
            .context_window_fallbacks
            .unwrap_or(&settings.context_window_fallbacks),
        content_policy: request
            .content_policy_fallbacks
            .unwrap_or(&settings.content_policy_fallbacks),
        budget: FallbackBudget {
            depth: request.depth,
            max_fallbacks: settings.max_fallbacks,
        },
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
    let has_unattempted = has_unattempted_targets(&found.targets, attempted);
    Some(ResolvedChain {
        targets: found.targets,
        has_unattempted,
    })
}

// LiteLLM stops before the attempt at or past max_fallbacks
#[must_use]
pub const fn advance_depth(budget: FallbackBudget) -> DepthOutcome {
    if budget.depth >= budget.max_fallbacks {
        return DepthOutcome::Exhausted;
    }
    DepthOutcome::Continue {
        next_depth: budget.depth.saturating_add(1),
    }
}
