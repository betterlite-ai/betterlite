use betterlite_config::sections::fallback_plans as config;
use betterlite_router::fallback_plans::plan::FallbackPlans;

mod tests;

// The config crate resolves the fallback keys once at load; the router only reads the typed plans
#[must_use]
pub fn plans(section: &config::Settings) -> FallbackPlans {
    FallbackPlans {
        fallbacks: section.fallbacks.clone(),
        context_window_fallbacks: section.context_window_fallbacks.clone(),
        content_policy_fallbacks: section.content_policy_fallbacks.clone(),
        max_fallbacks: section.max_fallbacks,
    }
}
