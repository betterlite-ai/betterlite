use std::collections::BTreeMap;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::adaptive_classification::classifier::Classifier;
use crate::adaptive_classification::config::AdaptiveRouterPreferences;
use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::REQUEST_TYPES;
use crate::adaptive_selection::default_preferences;
use crate::bandit_math::BanditCell;
use crate::bandit_math::initial_cell;
use crate::bandit_math::sampler::QualitySampler;

use super::error::AdaptivePlacementError;
use super::facts::ModelFacts;

// the AdaptiveRouter instance _ensure_adaptive_router builds, minus the persistence it drives
pub(super) struct AdaptiveState {
    pub(super) model_tiers: IndexMap<String, Vec<String>>,
    pub(super) preferences: IndexMap<String, AdaptiveRouterPreferences>,
    pub(super) costs: IndexMap<String, Decimal>,
    pub(super) cells: IndexMap<RequestType, IndexMap<String, BanditCell>>,
    pub(super) classifier: Classifier,
    pub(super) sampler: QualitySampler,
}

impl AdaptiveState {
    pub(super) fn new(
        available_models: &[String],
        pools: &BTreeMap<String, Vec<String>>,
        facts: &ModelFacts,
        sampler: QualitySampler,
    ) -> Result<Self, AdaptivePlacementError> {
        let model_tiers = tier_membership(available_models, pools);
        let mut preferences = IndexMap::with_capacity(available_models.len());
        let mut costs = IndexMap::with_capacity(available_models.len());
        for model in available_models {
            preferences.insert(
                model.clone(),
                facts
                    .preferences
                    .get(model)
                    .cloned()
                    .unwrap_or_else(default_preferences),
            );
            costs.insert(
                model.clone(),
                facts.costs.get(model).copied().unwrap_or(Decimal::ZERO),
            );
        }
        let mut cells = IndexMap::with_capacity(REQUEST_TYPES.len());
        for request_type in REQUEST_TYPES {
            let mut per_model = IndexMap::with_capacity(preferences.len());
            for (model, preference) in &preferences {
                per_model.insert(model.clone(), initial_cell(preference, request_type)?);
            }
            cells.insert(request_type, per_model);
        }
        Ok(Self {
            model_tiers,
            preferences,
            costs,
            cells,
            classifier: Classifier::new()?,
            sampler,
        })
    }
}

// _model_tiers: every pool a model sits in, in the router's tier order
fn tier_membership(
    available_models: &[String],
    pools: &BTreeMap<String, Vec<String>>,
) -> IndexMap<String, Vec<String>> {
    let mut model_tiers = IndexMap::with_capacity(available_models.len());
    for model in available_models {
        let tiers = pools
            .iter()
            .filter(|(_tier, models)| models.contains(model))
            .map(|(tier, _models)| tier.clone())
            .collect();
        model_tiers.insert(model.clone(), tiers);
    }
    model_tiers
}
