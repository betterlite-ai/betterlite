use crate::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;

use super::phase::IdentityPhase;
use super::write_transactional::ResetWrite;

// LITELLM_PROXY_BUDGET_NAME: the internal user that carries the proxy-wide budget, whose accumulator resets with it.
pub(crate) const PROXY_BUDGET_USER_ID: &str = "litellm-proxy-budget";

#[must_use]
pub(crate) fn accumulator_cache_keys(phase: IdentityPhase, writes: &[ResetWrite]) -> Vec<String> {
    let proxy_reset = phase == IdentityPhase::Users
        && writes
            .iter()
            .any(|write| write.identifier == PROXY_BUDGET_USER_ID);
    if proxy_reset {
        return vec![GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()];
    }
    Vec::new()
}

mod tests;
