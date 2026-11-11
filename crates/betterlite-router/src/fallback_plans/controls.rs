use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;

use crate::fallback_plans::groups::RequestMetadata;
use crate::fallback_plans::plan::{FallbackBudget, RequestOverrides};

// The per-request fallback overrides every streaming attempt must see again
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FallbackControls<'request> {
    pub fallbacks: Option<&'request [FallbackChainEntry]>,
    pub context_window_fallbacks: Option<&'request [FallbackChainEntry]>,
    pub content_policy_fallbacks: Option<&'request [FallbackChainEntry]>,
}

// The request one streaming attempt re-enters the fallback chain with
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopRequest<'request> {
    pub model: &'request str,
    pub controls: FallbackControls<'request>,
    pub budget: FallbackBudget,
    pub metadata: RequestMetadata,
}

#[must_use]
pub const fn per_request_fallback_controls<'request>(
    overrides: &RequestOverrides<'request>,
) -> FallbackControls<'request> {
    FallbackControls {
        fallbacks: overrides.fallbacks,
        context_window_fallbacks: overrides.context_window_fallbacks,
        content_policy_fallbacks: overrides.content_policy_fallbacks,
    }
}

// The hop copies the metadata buckets so an attempt write stays on its own attempt, and the caller keeps the attempted targets
#[must_use]
pub fn mid_stream_hop<'request>(
    model: &'request str,
    controls: FallbackControls<'request>,
    live: &RequestMetadata,
    budget: FallbackBudget,
) -> HopRequest<'request> {
    HopRequest {
        model,
        controls,
        budget,
        metadata: live.clone(),
    }
}
