use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_config::sections::adaptive_classification::Weights;
use betterlite_config::sections::complexity_adaptive_placement as section;
use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;
use betterlite_models::decisions::Json;
use betterlite_router::bandit_math::error::BanditMathError;
use betterlite_router::bandit_math::sampler::{QualitySampler, ScriptedSampler};
use betterlite_router::complexity_adaptive_placement::error::AdaptivePlacementError;
use betterlite_router::complexity_adaptive_placement::facts::ModelFacts;
use betterlite_router::complexity_adaptive_placement::pick::{PickRequest, Placed};
use betterlite_router::complexity_adaptive_placement::settings::PlacementSettings;
use betterlite_router::complexity_adaptive_placement::{AdaptivePlacement, PlacementSnapshot};
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::tier_definitions::model::TierSet;
use rust_decimal::Decimal;

use self::world::{World, tier};
use super::wire::{DecisionOutput, Fields, Payload, PreferenceOutput};
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod world;

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

impl From<AdaptivePlacementError> for OperationError {
    fn from(error: AdaptivePlacementError) -> Self {
        match error {
            AdaptivePlacementError::Bandit(BanditMathError::SamplesExhausted) => {
                Self::SamplesExhausted {
                    message: BanditMathError::SamplesExhausted.to_string(),
                }
            }
            AdaptivePlacementError::Bandit(other) => Self::InvalidRequest {
                message: other.to_string(),
            },
            refused @ (AdaptivePlacementError::NoTierPools
            | AdaptivePlacementError::EmptyTierPool { .. }
            | AdaptivePlacementError::AdaptiveDisabled
            | AdaptivePlacementError::MissingCell { .. }
            | AdaptivePlacementError::UnknownTier { .. }
            | AdaptivePlacementError::NonFinite { .. }
            | AdaptivePlacementError::Classification(_)
            | AdaptivePlacementError::Selection(_)
            | AdaptivePlacementError::Placement(_)) => Self::InvalidRequest {
                message: refused.to_string(),
            },
        }
    }
}

pub(super) fn number(text: &str, field: &str) -> Result<f64, OperationError> {
    text.parse::<f64>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a number: {text}"),
        })
}

pub(super) fn decimal(text: &str, field: &str) -> Result<Decimal, OperationError> {
    text.parse::<Decimal>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a decimal: {text}"),
        })
}

pub(super) fn fields_for(
    payload: &Payload,
    random: &mut InjectedRandom<'_>,
) -> Result<Fields, OperationError> {
    let world = World::new(&payload.pools, payload.default_model.as_deref())?;
    let settings = settings(payload)?;
    let facts = world::facts(&payload.deployments)?;
    let samples = payload
        .samples
        .iter()
        .map(|text| number(text, "sample"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut router = service(settings, &facts, &world.tiers, samples)?;
    warm(&mut router, payload)?;
    let context = world.context();
    let placement = world.placement(&context);
    let classified = tier(&payload.classified_tier);
    let floor = payload.hard_floor.as_deref().map(tier);
    let ceiling = payload.hard_ceiling.as_deref().map(tier);
    let fit = fit_filter(payload);
    let mut kwargs = payload.kwargs_present.unwrap_or(true).then(JsonMap::new);
    let request = PickRequest {
        classified_tier: &classified,
        user_message: &payload.user_message,
        hard_floor: floor.as_ref(),
        hard_ceiling: ceiling.as_ref(),
        fit_filter: fit.as_ref(),
    };
    let placed = router.pick(&request, &placement, kwargs.as_mut(), random)?;
    let snapshot = router.snapshot();
    fields_from(&placed, kwargs.as_ref(), snapshot.as_ref())
}

// the same litellm_params win model_info rule _ensure_adaptive_router applies
fn settings(payload: &Payload) -> Result<PlacementSettings, OperationError> {
    let weights = match payload.weights.as_ref() {
        Some(raw) => Some(Weights {
            quality: raw
                .quality
                .as_deref()
                .map(|text| number(text, "weights.quality"))
                .transpose()?,
            economy: raw
                .cost
                .as_deref()
                .map(|text| number(text, "weights.cost"))
                .transpose()?,
        }),
        None => None,
    };
    let penalty = payload
        .tier_distance_penalty
        .as_deref()
        .map(|text| number(text, "tier_distance_penalty"))
        .transpose()?
        .unwrap_or(section::DEFAULT_TIER_DISTANCE_PENALTY);
    Ok(PlacementSettings::resolve(&section::Config {
        adaptive: payload.adaptive,
        weights,
        tier_distance_penalty: penalty,
        eligible: eligible(payload)?,
    })?)
}

fn eligible(payload: &Payload) -> Result<AdaptiveEligible, OperationError> {
    match payload.adaptive_eligible.as_deref().unwrap_or("all") {
        "all" => Ok(AdaptiveEligible::All),
        "classified_tier" => Ok(AdaptiveEligible::ClassifiedTier),
        other => Err(OperationError::InvalidRequest {
            message: format!("adaptive_eligible is not a declared value: {other}"),
        }),
    }
}

fn fit_filter(payload: &Payload) -> Option<BTreeSet<String>> {
    payload
        .fit_filter
        .as_ref()
        .map(|models| models.iter().cloned().collect())
}

fn service(
    settings: PlacementSettings,
    facts: &ModelFacts,
    tiers: &TierSet,
    samples: Vec<f64>,
) -> Result<AdaptivePlacement, OperationError> {
    let draws = QualitySampler::Scripted(Box::new(ScriptedSampler {
        samples,
        position: 0,
    }));
    Ok(AdaptivePlacement::new(settings, tiers, facts, draws)?)
}

// the case's learned cells: apply_delta over the cold-start prior, as the bandit's updates do
fn warm(router: &mut AdaptivePlacement, payload: &Payload) -> Result<(), OperationError> {
    for observation in &payload.observations {
        router.observe(
            observation.request_type,
            &observation.model,
            number(&observation.delta_alpha, "delta_alpha")?,
            number(&observation.delta_beta, "delta_beta")?,
        )?;
    }
    Ok(())
}

fn fields_from(
    placed: &Placed,
    kwargs: Option<&JsonMap>,
    snapshot: Option<&PlacementSnapshot>,
) -> Result<Fields, OperationError> {
    Ok(Fields {
        chosen_model: placed.model.clone(),
        decision: decision_output(kwargs)?,
        decision_keys: decision_keys(kwargs),
        preferences: preferences(snapshot),
    })
}

// the decision the router rendered under metadata, not the typed value that produced it
fn decision_output(kwargs: Option<&JsonMap>) -> Result<Option<DecisionOutput>, OperationError> {
    let Some(Json::Object(metadata)) = kwargs.and_then(|storage| storage.get("metadata")) else {
        return Ok(None);
    };
    let Some(decision) = metadata.get("adaptive_router_decision") else {
        return Ok(None);
    };
    DecisionOutput::from_json(decision).map(Some)
}

// _ensure_adaptive_router's model_to_prefs, so a configured declaration is distinguishable
fn preferences(snapshot: Option<&PlacementSnapshot>) -> BTreeMap<String, PreferenceOutput> {
    let Some(known) = snapshot else {
        return BTreeMap::new();
    };
    let mut preferences = BTreeMap::new();
    for (model, preference) in &known.preferences {
        preferences.insert(
            model.clone(),
            PreferenceOutput {
                quality_tier: preference.quality_tier,
                strengths: preference
                    .strengths
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            },
        );
    }
    preferences
}

// the keys the router actually wrote under metadata, so a missing or extra one shows
fn decision_keys(kwargs: Option<&JsonMap>) -> Vec<String> {
    let Some(Json::Object(metadata)) = kwargs.and_then(|storage| storage.get("metadata")) else {
        return Vec::new();
    };
    let Some(Json::Object(decision)) = metadata.get("adaptive_router_decision") else {
        return Vec::new();
    };
    let mut keys: Vec<String> = decision.keys().cloned().collect();
    keys.sort();
    keys
}
