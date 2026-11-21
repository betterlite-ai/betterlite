use std::collections::BTreeMap;

use betterlite_config::sections::cache_cost_routing as section;
use betterlite_router::cache_cost_routing::{CacheCostRouting, CachedModelChooser, TierParams};
use betterlite_router::complexity_placement::TierPlacement;
use betterlite_router::complexity_placement::params::params_for_model as tier_params;
use betterlite_router::complexity_scoring::tier::{ComplexityTier, Tier};
use betterlite_router::router_predicates::JsonMap;

mod tests;

// _litellm_params_for_model over the placement's tier set: a tier entry's params for one model
pub struct PlacedTierParams<'deps> {
    pub placement: &'deps TierPlacement<'deps>,
}

impl TierParams for PlacedTierParams<'_> {
    fn params_for_model(&self, tier: &str, model: &str) -> JsonMap {
        let resolved = ComplexityTier::built_in(tier)
            .map_or_else(|| Tier::Named(tier.to_owned()), Tier::BuiltIn);
        tier_params(self.placement, Some(&resolved), model)
    }
}

// the hook as the router assembles it: the cache-aware settings, the tier label map, the tier lookup and B3124's chooser
pub struct CacheCostRoutingService<'deps> {
    params: PlacedTierParams<'deps>,
    config: &'deps section::Config,
    tier_labels: &'deps BTreeMap<ComplexityTier, String>,
    chooser: &'deps dyn CachedModelChooser,
}

impl<'deps> CacheCostRoutingService<'deps> {
    #[must_use]
    pub fn new(
        config: &'deps section::Config,
        tier_labels: &'deps BTreeMap<ComplexityTier, String>,
        placement: &'deps TierPlacement<'deps>,
        chooser: &'deps dyn CachedModelChooser,
    ) -> Self {
        Self {
            params: PlacedTierParams { placement },
            config,
            tier_labels,
            chooser,
        }
    }

    // the service borrows the tier lookup, so it lives as long as the borrow of this adapter
    #[must_use]
    pub fn routing(&self) -> CacheCostRouting<'_> {
        CacheCostRouting {
            config: self.config,
            tier_labels: self.tier_labels,
            params_for_model: &self.params,
            chooser: self.chooser,
        }
    }
}
