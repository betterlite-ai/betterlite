pub mod costs;
pub mod error;
pub mod pre_routing;

mod tests;

use betterlite_models::decisions::Json;
use betterlite_prompts::message_fields::last_user::get_last_user_message;
use indexmap::{IndexMap, IndexSet};
use rust_decimal::Decimal;

use crate::adaptive_classification::classifier::Classifier;
use crate::adaptive_classification::config::{
    AdaptiveRouterConfig, AdaptiveRouterPreferences, AdaptiveRouterWeights,
};
use crate::adaptive_classification::constants::CHOSEN_MODEL_KEY;
use crate::adaptive_classification::minimum_quality::minimum_quality_tier;
use crate::adaptive_classification::request_type::RequestType;
use crate::bandit_math::sampler::QualitySampler;
use crate::bandit_math::{BanditCell, Candidate, initial_cell, pick_best};

use self::costs::normalized_cost;
use self::error::AdaptiveSelectionError;
use self::pre_routing::{
    PreRoutingRequest, PreRoutingResponse, RouterKind, RoutingCause, RoutingDecision,
};

const DEFAULT_QUALITY_TIER: u8 = 2;

pub const REQUEST_TYPES: [RequestType; 7] = [
    RequestType::CodeGeneration,
    RequestType::CodeUnderstanding,
    RequestType::TechnicalDesign,
    RequestType::AnalyticalReasoning,
    RequestType::Writing,
    RequestType::FactualLookup,
    RequestType::General,
];

#[must_use]
pub const fn default_preferences() -> AdaptiveRouterPreferences {
    AdaptiveRouterPreferences {
        quality_tier: DEFAULT_QUALITY_TIER,
        strengths: Vec::new(),
    }
}

pub struct AdaptiveSelection {
    router_name: String,
    available_models: Vec<String>,
    weights: AdaptiveRouterWeights,
    preferences: IndexMap<String, AdaptiveRouterPreferences>,
    model_costs: IndexMap<String, Decimal>,
    cells: IndexMap<RequestType, IndexMap<String, BanditCell>>,
    classifier: Classifier,
    sampler: QualitySampler,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateSnapshot {
    pub router_name: String,
    pub available_models: Vec<String>,
    pub weights: AdaptiveRouterWeights,
    pub model_costs: Vec<(String, Decimal)>,
    pub cells: Vec<SnapshotCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotCell {
    pub request_type: RequestType,
    pub model: String,
    pub alpha: f64,
    pub beta: f64,
    pub samples: u64,
    pub quality_mean: f64,
}

impl AdaptiveSelection {
    pub fn new(
        router_name: String,
        config: &AdaptiveRouterConfig,
        preferences: &IndexMap<String, AdaptiveRouterPreferences>,
        model_costs: &IndexMap<String, Decimal>,
        sampler: QualitySampler,
    ) -> Result<Self, AdaptiveSelectionError> {
        let mut cells = IndexMap::with_capacity(REQUEST_TYPES.len());
        for request_type in REQUEST_TYPES {
            let mut per_model = IndexMap::with_capacity(config.available_models.len());
            for model in &config.available_models {
                let prior = preferences
                    .get(model)
                    .cloned()
                    .unwrap_or_else(default_preferences);
                per_model.insert(model.clone(), initial_cell(&prior, request_type)?);
            }
            cells.insert(request_type, per_model);
        }
        Ok(Self {
            router_name,
            available_models: config.available_models.clone(),
            weights: config.weights,
            preferences: preferences.clone(),
            model_costs: model_costs.clone(),
            cells,
            classifier: Classifier::new()?,
            sampler,
        })
    }

    pub fn pick_model(
        &mut self,
        request_type: RequestType,
        min_quality_tier: Option<i64>,
    ) -> Result<String, AdaptiveSelectionError> {
        let mut eligible = self.eligible_models(min_quality_tier);
        let mut seen = IndexSet::new();
        eligible.retain(|model| seen.insert(model.clone()));
        if eligible.is_empty() {
            let tier = min_quality_tier.map_or_else(|| "None".to_owned(), |tier| tier.to_string());
            return Err(AdaptiveSelectionError::NoEligibleModels {
                router_name: self.router_name.clone(),
                tier,
            });
        }
        let all_costs = eligible
            .iter()
            .map(|model| self.model_cost(model))
            .collect::<Vec<_>>();
        let mut candidates = Vec::with_capacity(eligible.len());
        for model in &eligible {
            let cell = self
                .cells
                .get(&request_type)
                .and_then(|cells| cells.get(model))
                .copied()
                .ok_or_else(|| AdaptiveSelectionError::MissingCell {
                    request_type,
                    model: model.clone(),
                })?;
            let economy_score = normalized_cost(self.model_cost(model), &all_costs)?;
            candidates.push(Candidate {
                model,
                cell,
                economy_score,
            });
        }
        Ok(pick_best(&candidates, self.weights, &mut self.sampler)?)
    }

    #[must_use]
    pub fn state_snapshot(&self) -> StateSnapshot {
        let mut cells = Vec::new();
        for (request_type, per_model) in &self.cells {
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
        StateSnapshot {
            router_name: self.router_name.clone(),
            available_models: self.available_models.clone(),
            weights: self.weights,
            model_costs: self
                .model_costs
                .iter()
                .map(|(model, cost)| (model.clone(), *cost))
                .collect(),
            cells,
        }
    }

    pub fn pre_route<'messages>(
        &mut self,
        request: PreRoutingRequest<'messages, '_>,
    ) -> Result<PreRoutingResponse<'messages>, AdaptiveSelectionError> {
        let PreRoutingRequest {
            messages,
            headers,
            metadata,
        } = request;
        let text = messages.and_then(get_last_user_message).unwrap_or_default();
        let request_type = self.classifier.classify_prompt(&text);
        let min_quality_tier = minimum_quality_tier(headers, metadata.as_ref());
        let chosen_model = self.pick_model(request_type, min_quality_tier)?;
        let next_metadata = match metadata.take() {
            Some(Json::Object(mut fields)) => {
                fields.insert(
                    CHOSEN_MODEL_KEY.to_owned(),
                    Json::Text(chosen_model.clone()),
                );
                Json::Object(fields)
            }
            Some(other) => other,
            None => Json::Object(IndexMap::from_iter([(
                CHOSEN_MODEL_KEY.to_owned(),
                Json::Text(chosen_model.clone()),
            )])),
        };
        *metadata = Some(next_metadata);
        let routing_decision = RoutingDecision {
            router_model_name: self.router_name.clone(),
            router_type: RouterKind::Adaptive,
            routed_model: chosen_model.clone(),
            cause: RoutingCause::Bandit,
            request_type,
        };
        Ok(PreRoutingResponse {
            model: chosen_model,
            messages,
            routing_decision,
        })
    }

    fn eligible_models(&self, min_quality_tier: Option<i64>) -> Vec<String> {
        let Some(minimum) = min_quality_tier else {
            return self.available_models.clone();
        };
        self.available_models
            .iter()
            .filter(|model| {
                let tier = self
                    .preferences
                    .get(*model)
                    .map_or(DEFAULT_QUALITY_TIER, |stored| stored.quality_tier);
                i64::from(tier) >= minimum
            })
            .cloned()
            .collect()
    }

    fn model_cost(&self, model: &str) -> Decimal {
        self.model_costs
            .get(model)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }
}
