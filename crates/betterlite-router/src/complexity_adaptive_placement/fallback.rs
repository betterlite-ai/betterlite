use std::collections::BTreeSet;

use rand::Rng;

use crate::complexity_placement::TierPlacement;
use crate::complexity_placement::pools::{model_for_tier, pick_from_names};
use crate::complexity_scoring::tier::Tier;

use super::AdaptivePlacement;
use super::error::AdaptivePlacementError;
use super::pick::{PickRequest, Placed};

// the collaborators a placement fallback needs: the request, its tier placement and the draw source
pub(super) struct Fallback<'request, 'tier, 'deps, R: Rng + ?Sized> {
    pub(super) request: &'request PickRequest<'tier>,
    pub(super) placement: &'request TierPlacement<'deps>,
    pub(super) random: &'request mut R,
}

// The type's one inherent impl is in the module root; multiple_inherent_impl keeps the fallback free here.
pub(super) fn fitting_fallback<R>(
    service: &AdaptivePlacement,
    fallback: Fallback<'_, '_, '_, R>,
) -> Result<Placed, AdaptivePlacementError>
where
    R: Rng + ?Sized,
{
    // _fitting_tier_fallback: a context filter picks from the fitting names, else the tier does
    let Fallback {
        request,
        placement,
        random,
    } = fallback;
    let fitting = fitting_models(service, request.classified_tier, request.fit_filter);
    let model = if request.fit_filter.is_some() && !fitting.is_empty() {
        pick_from_names(&fitting, request.classified_tier.name(), random)?
    } else {
        model_for_tier(placement, request.classified_tier, random)?
    };
    Ok(Placed {
        model,
        decision: None,
    })
}

fn fitting_models(
    service: &AdaptivePlacement,
    tier: &Tier,
    fit_filter: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let Some(models) = service.pools.get(tier.name()) else {
        return Vec::new();
    };
    models
        .iter()
        .filter(|model| fit_filter.is_none_or(|fit| fit.contains(*model)))
        .cloned()
        .collect()
}
