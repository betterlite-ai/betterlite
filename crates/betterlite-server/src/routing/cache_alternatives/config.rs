use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::tier_definitions::model::TierSet;

// The complexity router state cache-aware selection reads: the active tiers and the settings that refuse it.
#[derive(Debug, Clone, Copy)]
pub struct CacheRoutingConfig<'config> {
    pub tiers: &'config TierSet,
    pub features: &'config [Feature],
}
