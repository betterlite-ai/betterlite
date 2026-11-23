use crate::complexity_adaptive_placement::AdaptivePlacement;
use crate::complexity_adaptive_placement::error::AdaptivePlacementError;
use crate::complexity_adaptive_placement::facts::ModelFacts;
use crate::complexity_adaptive_placement::settings::PlacementSettings;

use super::{scripted_sampler, tier_set};

pub(crate) fn adaptive_service(
    settings: PlacementSettings,
    pools: &[(&str, &[&str])],
    facts: &ModelFacts,
    samples: &[f64],
) -> Result<AdaptivePlacement, AdaptivePlacementError> {
    let tiers = tier_set(pools);
    AdaptivePlacement::new(settings, &tiers, facts, scripted_sampler(samples))
}
