use std::collections::BTreeSet;

use rand::Rng;

use crate::adaptive_classification::request_type::RequestType;
use crate::complexity_placement::error::TierPlacementError;
use crate::complexity_placement::pools::draw_index;
use crate::router_predicates::JsonMap;

use super::AdaptivePlacement;
use super::decision::{AdaptiveDecision, ColdStartCandidate, ColdStartDecision, write_decision};
use super::error::AdaptivePlacementError;
use super::pick::{Placed, cell_of};
use super::state::AdaptiveState;

// the cold-start plan: the classified pool's models that carry no observation yet
pub(super) struct ColdStartPlan {
    pub(super) request_type: RequestType,
    pub(super) classified_tier: String,
    pub(super) candidates: Vec<ColdStartCandidate>,
}

// multiple_inherent_impl keeps the cold start free; L3055-3075: while a pool model is unobserved, that pool decides
pub(super) fn cold_start_candidates(
    placement: &AdaptivePlacement,
    state: &AdaptiveState,
    classified_tier: &str,
    request_type: RequestType,
    fit_filter: Option<&BTreeSet<String>>,
) -> Result<Vec<ColdStartCandidate>, AdaptivePlacementError> {
    let Some(models) = placement.pools.get(classified_tier) else {
        return Ok(Vec::new());
    };
    let mut candidates = Vec::new();
    for model in models {
        if fit_filter.is_some_and(|fit| !fit.contains(model)) {
            continue;
        }
        let samples = cell_of(state, request_type, model)?.total_samples();
        if samples == 0 {
            candidates.push(ColdStartCandidate {
                model: model.clone(),
                total_samples: samples,
            });
        }
    }
    Ok(candidates)
}

// random.choice over the cold candidates, recorded as the classified_tier decision
pub(super) fn cold_start_outcome<R>(
    placement: &AdaptivePlacement,
    plan: ColdStartPlan,
    request_kwargs: Option<&mut JsonMap>,
    random: &mut R,
) -> Result<Placed, AdaptivePlacementError>
where
    R: Rng + ?Sized,
{
    let index = draw_index(plan.candidates.len(), random)?;
    let Some(candidate) = plan.candidates.get(index) else {
        return Err(AdaptivePlacementError::Placement(TierPlacementError::Draw));
    };
    let chosen_model = candidate.model.clone();
    let decision = AdaptiveDecision::ColdStart(ColdStartDecision {
        classified_tier: plan.classified_tier,
        request_type: plan.request_type,
        weights: placement.settings.weights,
        tier_distance_penalty: placement.settings.tier_distance_penalty,
        chosen_model: chosen_model.clone(),
        candidates: plan.candidates,
    });
    write_decision(request_kwargs, &decision)?;
    Ok(Placed {
        model: chosen_model,
        decision: Some(decision),
    })
}
