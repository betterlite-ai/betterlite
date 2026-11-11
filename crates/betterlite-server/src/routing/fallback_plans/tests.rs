#![cfg(test)]

use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use indexmap::IndexMap;

use super::plans;

#[test]
fn the_router_plan_carries_every_configured_chain() {
    let chain = FallbackChainEntry::Group(IndexMap::from([(
        String::from("gpt-4"),
        vec![FallbackTarget::Model(String::from("claude"))],
    )]));
    let settings = Settings {
        fallbacks: vec![chain.clone()],
        context_window_fallbacks: vec![chain.clone()],
        content_policy_fallbacks: vec![chain],
        max_fallbacks: 3,
    };
    let resolved = plans(&settings);
    assert_eq!(resolved.fallbacks, settings.fallbacks);
    assert_eq!(
        resolved.context_window_fallbacks,
        settings.context_window_fallbacks
    );
    assert_eq!(
        resolved.content_policy_fallbacks,
        settings.content_policy_fallbacks
    );
    assert_eq!(resolved.max_fallbacks, 3);
}
