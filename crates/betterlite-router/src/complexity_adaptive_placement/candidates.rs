use std::collections::BTreeSet;

use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;

use crate::complexity_scoring::tier::Tier;

use super::AdaptivePlacement;
use super::error::AdaptivePlacementError;

// _adaptive_candidate_models' arguments: the classified tier, the plan's bounds and the context fit
pub struct CandidateQuery<'request> {
    pub classified_tier: &'request Tier,
    pub hard_floor: Option<&'request Tier>,
    pub hard_ceiling: Option<&'request Tier>,
    pub fit_filter: Option<&'request BTreeSet<String>>,
}

// The type's one inherent impl is in the module root; multiple_inherent_impl keeps the query free here.
pub(super) fn severity_name(
    placement: &AdaptivePlacement,
    name: &str,
) -> Result<u32, AdaptivePlacementError> {
    let position = placement.tier_names.iter().position(|tier| tier == name);
    position
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| AdaptivePlacementError::UnknownTier {
            tier: name.to_owned(),
        })
}

// a model is eligible if at least one of its pools satisfies a bound
pub(super) fn severities(
    placement: &AdaptivePlacement,
    model: &str,
    classified_tier: &Tier,
) -> Result<Vec<u32>, AdaptivePlacementError> {
    let Some(state) = placement.state.as_ref() else {
        return Ok(vec![severity_name(placement, classified_tier.name())?]);
    };
    let Some(tiers) = state.model_tiers.get(model) else {
        return Ok(vec![severity_name(placement, classified_tier.name())?]);
    };
    tiers
        .iter()
        .map(|tier| severity_name(placement, tier))
        .collect()
}

pub(super) fn distance(
    placement: &AdaptivePlacement,
    model: &str,
    classified_tier: &Tier,
    classified_severity: u32,
) -> Result<u32, AdaptivePlacementError> {
    if placement.settings.eligible == AdaptiveEligible::ClassifiedTier {
        return Ok(0);
    }
    let model_severities = severities(placement, model, classified_tier)?;
    Ok(model_severities
        .iter()
        .map(|level| level.abs_diff(classified_severity))
        .min()
        .unwrap_or(0))
}
