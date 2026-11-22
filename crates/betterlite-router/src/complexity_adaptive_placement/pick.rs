use std::collections::BTreeSet;

use rand::Rng;
use rust_decimal::Decimal;

use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::costs::normalized_cost;
use crate::bandit_math::BanditCell;
use crate::complexity_scoring::tier::Tier;
use crate::router_predicates::JsonMap;
use crate::soft_floor_math::placement_score;

use super::AdaptivePlacement;
use super::candidates::{CandidateQuery, distance, severity_name};
use super::cold_start::{ColdStartPlan, cold_start_candidates};
use super::decision::{AdaptiveChoice, AdaptiveDecision, CandidateScore, write_decision};
use super::error::AdaptivePlacementError;
use super::fallback::{Fallback, fitting_fallback};
use super::state::AdaptiveState;

// _soft_floor_pick's arguments besides the router: the tier, the transient bounds and the message
pub struct PickRequest<'request> {
    pub classified_tier: &'request Tier,
    pub user_message: &'request str,
    pub hard_floor: Option<&'request Tier>,
    pub hard_ceiling: Option<&'request Tier>,
    pub fit_filter: Option<&'request BTreeSet<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub model: String,
    // Some only when an adaptive phase placed the model; the tier fallbacks record nothing
    pub decision: Option<AdaptiveDecision>,
}

pub(super) enum Prepared {
    ColdStart(ColdStartPlan),
    Adaptive(AdaptivePlan),
    Fallback,
}

pub(super) struct AdaptivePlan {
    request_type: RequestType,
    classified_tier: String,
    candidates: Vec<String>,
    all_costs: Vec<Decimal>,
    distances: Vec<u32>,
}

#[derive(Default)]
struct Sampled {
    chosen: Option<usize>,
    candidates: Vec<CandidateScore>,
}

// The type's one inherent impl is in the module root; multiple_inherent_impl keeps the plan free here.
pub(super) fn prepare(
    placement: &AdaptivePlacement,
    request: &PickRequest<'_>,
) -> Result<Prepared, AdaptivePlacementError> {
    let Some(state) = placement.state.as_ref() else {
        return Ok(Prepared::Fallback);
    };
    let request_type = state.classifier.classify_prompt(request.user_message);
    let classified_tier = request.classified_tier.name().to_owned();
    let cold_start = cold_start_candidates(
        placement,
        state,
        &classified_tier,
        request_type,
        request.fit_filter,
    )?;
    if cold_start.is_empty() {
        return adaptive_plan(placement, state, request, request_type, classified_tier);
    }
    Ok(Prepared::ColdStart(ColdStartPlan {
        request_type,
        classified_tier,
        candidates: cold_start,
    }))
}

// the adaptive plan: the bounded candidates, the broader set's costs and each tier distance
fn adaptive_plan(
    placement: &AdaptivePlacement,
    state: &AdaptiveState,
    request: &PickRequest<'_>,
    request_type: RequestType,
    classified_tier: String,
) -> Result<Prepared, AdaptivePlacementError> {
    let classified_severity = severity_name(placement, request.classified_tier.name())?;
    let tier = request.classified_tier;
    let broader = placement.candidate_models(&CandidateQuery {
        classified_tier: tier,
        hard_floor: None,
        hard_ceiling: None,
        fit_filter: request.fit_filter,
    })?;
    let bounded = placement.candidate_models(&CandidateQuery {
        classified_tier: tier,
        hard_floor: request.hard_floor,
        hard_ceiling: request.hard_ceiling,
        fit_filter: request.fit_filter,
    })?;
    if bounded.is_empty() {
        return Ok(Prepared::Fallback);
    }
    let mut all_costs = Vec::with_capacity(broader.len());
    for model in &broader {
        all_costs.push(state.costs.get(model).copied().unwrap_or(Decimal::ZERO));
    }
    let mut distances = Vec::with_capacity(bounded.len());
    for model in &bounded {
        distances.push(distance(placement, model, tier, classified_severity)?);
    }
    Ok(Prepared::Adaptive(AdaptivePlan {
        request_type,
        classified_tier,
        candidates: bounded,
        all_costs,
        distances,
    }))
}

pub(super) fn adaptive_outcome<R>(
    placement: &mut AdaptivePlacement,
    plan: AdaptivePlan,
    fallback: Fallback<'_, '_, '_, R>,
    request_kwargs: Option<&mut JsonMap>,
) -> Result<Placed, AdaptivePlacementError>
where
    R: Rng + ?Sized,
{
    let sampled = sample(placement, &plan)?;
    let Some(index) = sampled.chosen else {
        return fitting_fallback(placement, fallback);
    };
    let Some(chosen_model) = sampled
        .candidates
        .get(index)
        .map(|candidate| candidate.model.clone())
    else {
        return fitting_fallback(placement, fallback);
    };
    let decision = AdaptiveDecision::Adaptive(AdaptiveChoice {
        classified_tier: plan.classified_tier,
        request_type: plan.request_type,
        eligible: placement.settings.eligible,
        weights: placement.settings.weights,
        tier_distance_penalty: placement.settings.tier_distance_penalty,
        chosen_model: chosen_model.clone(),
        candidates: sampled.candidates,
    });
    write_decision(request_kwargs, &decision)?;
    Ok(Placed {
        model: chosen_model,
        decision: Some(decision),
    })
}

// one Thompson draw per bounded candidate, scored with its cost and tier distance
fn sample(
    placement: &mut AdaptivePlacement,
    plan: &AdaptivePlan,
) -> Result<Sampled, AdaptivePlacementError> {
    let settings = placement.settings;
    let Some(state) = placement.state.as_mut() else {
        return Ok(Sampled::default());
    };
    let mut candidates = Vec::with_capacity(plan.candidates.len());
    let mut best: Option<(usize, f64)> = None;
    for (index, model) in plan.candidates.iter().enumerate() {
        let cell = cell_of(state, plan.request_type, model)?;
        let quality_sample = state.sampler.sample(cell)?;
        let model_cost = state.costs.get(model).copied().unwrap_or(Decimal::ZERO);
        let economy_score = normalized_cost(model_cost, &plan.all_costs)?;
        let tier_distance = plan.distances.get(index).copied().unwrap_or(0);
        let score = placement_score(
            settings.weights,
            quality_sample,
            economy_score,
            settings.tier_distance_penalty,
            usize::try_from(tier_distance).unwrap_or(usize::MAX),
        )?;
        candidates.push(CandidateScore {
            model: model.clone(),
            quality_sample,
            economy_score,
            tier_distance,
            score,
        });
        // the first candidate keeps a tie, as the source's strict comparison does
        if best.is_none_or(|(_best_index, best_score)| score > best_score) {
            best = Some((index, score));
        }
    }
    Ok(Sampled {
        chosen: best.map(|(index, _score)| index),
        candidates,
    })
}

pub(super) fn cell_of(
    state: &AdaptiveState,
    request_type: RequestType,
    model: &str,
) -> Result<BanditCell, AdaptivePlacementError> {
    state
        .cells
        .get(&request_type)
        .and_then(|cells| cells.get(model))
        .copied()
        .ok_or_else(|| AdaptivePlacementError::MissingCell {
            request_type,
            model: model.to_owned(),
        })
}
