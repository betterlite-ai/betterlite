pub mod candidates;
mod cold_start;
pub mod decision;
pub mod error;
pub mod facts;
mod fallback;
pub mod pick;
pub mod settings;
mod state;

mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;
use indexmap::IndexSet;
use rand::Rng;
use rust_decimal::Decimal;

use crate::adaptive_classification::config::{AdaptiveRouterPreferences, AdaptiveRouterWeights};
use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::SnapshotCell;
use crate::bandit_math::apply_delta;
use crate::bandit_math::sampler::QualitySampler;
use crate::complexity_placement::TierPlacement;
use crate::complexity_placement::pools::tier_pools;
use crate::complexity_scoring::tier::Tier;
use crate::router_predicates::JsonMap;
use crate::tier_definitions::model::TierSet;

use self::candidates::{CandidateQuery, severities, severity_name};
use self::cold_start::cold_start_outcome;
use self::error::AdaptivePlacementError;
use self::facts::ModelFacts;
use self::fallback::{Fallback, fitting_fallback};
use self::pick::{PickRequest, Placed, Prepared, adaptive_outcome, prepare};
use self::settings::PlacementSettings;
use self::state::AdaptiveState;

// the ComplexityRouter's adaptive placement state: B334's pools, B339's cells and their lifecycle
pub struct AdaptivePlacement {
    settings: PlacementSettings,
    pools: BTreeMap<String, Vec<String>>,
    tier_names: Vec<String>,
    available_models: Vec<String>,
    state: Option<AdaptiveState>,
}

// _ensure_adaptive_router's construction, as a value a caller can compare and render
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementSnapshot {
    pub weights: AdaptiveRouterWeights,
    pub tier_distance_penalty: f64,
    pub eligible: AdaptiveEligible,
    pub available_models: Vec<String>,
    pub model_tiers: Vec<(String, Vec<String>)>,
    pub preferences: Vec<(String, AdaptiveRouterPreferences)>,
    pub costs: Vec<(String, Decimal)>,
    pub cells: Vec<SnapshotCell>,
}

// The type's one inherent impl: multiple_inherent_impl keeps each area's logic free in its module.
impl AdaptivePlacement {
    // _ensure_adaptive_router: one-shot state from the pools and their facts; adaptive=false builds none
    pub fn new(
        settings: PlacementSettings,
        tiers: &TierSet,
        facts: &ModelFacts,
        sampler: QualitySampler,
    ) -> Result<Self, AdaptivePlacementError> {
        let pools = tier_pools(tiers);
        let tier_names: Vec<String> = tiers.tier_names().into_iter().map(str::to_owned).collect();
        let available_models = available_models(&tier_names, &pools);
        let state = if settings.adaptive {
            validate_pools(&pools)?;
            Some(AdaptiveState::new(
                &available_models,
                &pools,
                facts,
                sampler,
            )?)
        } else {
            None
        };
        Ok(Self {
            settings,
            pools,
            tier_names,
            available_models,
            state,
        })
    }

    // the bandit update lifecycle B339 owns: apply_delta, capped and rejected when non-finite
    pub fn observe(
        &mut self,
        request_type: RequestType,
        model: &str,
        delta_alpha: f64,
        delta_beta: f64,
    ) -> Result<(), AdaptivePlacementError> {
        let Some(state) = self.state.as_mut() else {
            return Err(AdaptivePlacementError::AdaptiveDisabled);
        };
        let Some(cells) = state.cells.get_mut(&request_type) else {
            return Err(AdaptivePlacementError::MissingCell {
                request_type,
                model: model.to_owned(),
            });
        };
        let Some(cell) = cells.get_mut(model) else {
            return Err(AdaptivePlacementError::MissingCell {
                request_type,
                model: model.to_owned(),
            });
        };
        *cell = apply_delta(*cell, delta_alpha, delta_beta)?;
        Ok(())
    }

    // None when adaptive is off, as _ensure_adaptive_router returns None
    #[must_use]
    pub fn snapshot(&self) -> Option<PlacementSnapshot> {
        let state = self.state.as_ref()?;
        let mut cells = Vec::new();
        for (request_type, per_model) in &state.cells {
            for (model, cell) in per_model {
                cells.push(SnapshotCell {
                    request_type: *request_type,
                    model: model.clone(),
                    alpha: cell.alpha,
                    beta: cell.beta,
                    samples: cell.total_samples(),
                    quality_mean: cell.quality_mean(),
                });
            }
        }
        cells.sort_by_cached_key(|cell| (cell.request_type.to_string(), cell.model.clone()));
        Some(PlacementSnapshot {
            weights: self.settings.weights,
            tier_distance_penalty: self.settings.tier_distance_penalty,
            eligible: self.settings.eligible,
            available_models: self.available_models.clone(),
            model_tiers: state
                .model_tiers
                .iter()
                .map(|(model, tiers)| (model.clone(), tiers.clone()))
                .collect(),
            preferences: state
                .preferences
                .iter()
                .map(|(model, preferences)| (model.clone(), preferences.clone()))
                .collect(),
            costs: state
                .costs
                .iter()
                .map(|(model, cost)| (model.clone(), *cost))
                .collect(),
            cells,
        })
    }

    // _adaptive_candidate_models: the eligible pool, the context-fit filter, then the plan's bounds
    pub fn candidate_models(
        &self,
        query: &CandidateQuery<'_>,
    ) -> Result<Vec<String>, AdaptivePlacementError> {
        let floor = query
            .hard_floor
            .map(|tier| severity_name(self, tier.name()))
            .transpose()?;
        let ceiling = query
            .hard_ceiling
            .map(|tier| severity_name(self, tier.name()))
            .transpose()?;
        let base = match self.settings.eligible {
            AdaptiveEligible::ClassifiedTier => self
                .pools
                .get(query.classified_tier.name())
                .cloned()
                .unwrap_or_default(),
            AdaptiveEligible::All => self.available_models.clone(),
        };
        let mut candidates = Vec::with_capacity(base.len());
        for model in base {
            if query.fit_filter.is_some_and(|fit| !fit.contains(&model)) {
                continue;
            }
            let model_severities = severities(self, &model, query.classified_tier)?;
            if floor.is_some_and(|bound| !model_severities.iter().any(|level| *level >= bound)) {
                continue;
            }
            if ceiling.is_some_and(|bound| !model_severities.iter().any(|level| *level <= bound)) {
                continue;
            }
            candidates.push(model);
        }
        Ok(candidates)
    }

    // _soft_floor_pick: the cold-start pool first, then the sampled candidates, else the tier fallback
    pub fn pick<R>(
        &mut self,
        request: &PickRequest<'_>,
        placement: &TierPlacement<'_>,
        request_kwargs: Option<&mut JsonMap>,
        random: &mut R,
    ) -> Result<Placed, AdaptivePlacementError>
    where
        R: Rng + ?Sized,
    {
        let fallback = Fallback {
            request,
            placement,
            random,
        };
        // a custom tier name has no severity index, and no adaptive state means adaptive is off
        if self.state.is_none() || matches!(request.classified_tier, Tier::Named(_)) {
            return fitting_fallback(self, fallback);
        }
        match prepare(self, request)? {
            Prepared::ColdStart(plan) => {
                cold_start_outcome(self, plan, request_kwargs, fallback.random)
            }
            Prepared::Adaptive(plan) => adaptive_outcome(self, plan, fallback, request_kwargs),
            Prepared::Fallback => fitting_fallback(self, fallback),
        }
    }
}

// dict.fromkeys over the pools in ladder order, so a built-in config iterates as LiteLLM's YAML does
fn available_models(tier_names: &[String], pools: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut models: IndexSet<String> = IndexSet::new();
    let mut named: IndexSet<&str> = IndexSet::new();
    for tier in tier_names {
        named.insert(tier.as_str());
        if let Some(pool) = pools.get(tier) {
            models.extend(pool.iter().cloned());
        }
    }
    for (tier, pool) in pools {
        // insert reports whether the ladder already named this tier, so each pool is read once
        if named.insert(tier.as_str()) {
            models.extend(pool.iter().cloned());
        }
    }
    models.into_iter().collect()
}

// _validate_adaptive_pools: adaptive needs pools, and no configured pool may be empty
fn validate_pools(pools: &BTreeMap<String, Vec<String>>) -> Result<(), AdaptivePlacementError> {
    if !pools.values().any(|models| !models.is_empty()) {
        return Err(AdaptivePlacementError::NoTierPools);
    }
    for (tier, models) in pools {
        if models.is_empty() {
            return Err(AdaptivePlacementError::EmptyTierPool { tier: tier.clone() });
        }
    }
    Ok(())
}
