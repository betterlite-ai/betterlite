use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::complexity_adaptive_placement::decision::{
    AdaptiveChoice, AdaptiveDecision, ColdStartDecision,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    #[serde(default)]
    pub(super) adaptive: bool,
    pub(super) weights: Option<WeightsInput>,
    pub(super) tier_distance_penalty: Option<String>,
    pub(super) adaptive_eligible: Option<String>,
    #[serde(default)]
    pub(super) pools: Vec<PoolInput>,
    #[serde(default)]
    pub(super) deployments: Vec<DeploymentInput>,
    pub(super) default_model: Option<String>,
    pub(super) classified_tier: String,
    pub(super) user_message: String,
    pub(super) hard_floor: Option<String>,
    pub(super) hard_ceiling: Option<String>,
    pub(super) fit_filter: Option<Vec<String>>,
    #[serde(default)]
    pub(super) observations: Vec<ObservationInput>,
    #[serde(default)]
    pub(super) samples: Vec<String>,
    pub(super) kwargs_present: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WeightsInput {
    pub(super) quality: Option<String>,
    pub(super) cost: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PoolInput {
    pub(super) tier: String,
    pub(super) models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentInput {
    pub(super) model_name: String,
    pub(super) quality_tier: Option<u8>,
    #[serde(default)]
    pub(super) strengths: Vec<RequestType>,
    pub(super) params_cost: Option<String>,
    pub(super) info_cost: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservationInput {
    pub(super) request_type: RequestType,
    pub(super) model: String,
    pub(super) delta_alpha: String,
    pub(super) delta_beta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) chosen_model: String,
    pub(super) decision: Option<DecisionOutput>,
    pub(super) decision_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct DecisionOutput {
    pub(super) phase: String,
    pub(super) classified_tier: String,
    pub(super) request_type: String,
    pub(super) eligible_mode: String,
    pub(super) quality_weight: String,
    pub(super) cost_weight: String,
    pub(super) tier_distance_penalty: String,
    pub(super) chosen_model: String,
    pub(super) candidates: Vec<CandidateOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CandidateOutput {
    pub(super) model: String,
    pub(super) total_samples: Option<u64>,
    pub(super) quality_sample: Option<String>,
    pub(super) cost_score: Option<String>,
    pub(super) tier_distance: Option<u32>,
    pub(super) score: Option<String>,
}

impl DecisionOutput {
    pub(super) fn from_decision(decision: &AdaptiveDecision) -> Self {
        match decision {
            AdaptiveDecision::ColdStart(decision) => Self::from_cold_start(decision),
            AdaptiveDecision::Adaptive(choice) => Self::from_adaptive(choice),
        }
    }

    fn from_cold_start(decision: &ColdStartDecision) -> Self {
        Self {
            phase: "cold_start".to_owned(),
            classified_tier: decision.classified_tier.clone(),
            request_type: decision.request_type.to_string(),
            eligible_mode: "classified_tier".to_owned(),
            quality_weight: format!("{:.6}", decision.weights.quality),
            cost_weight: format!("{:.6}", decision.weights.economy),
            tier_distance_penalty: format!("{:.6}", decision.tier_distance_penalty),
            chosen_model: decision.chosen_model.clone(),
            candidates: decision
                .candidates
                .iter()
                .map(|candidate| CandidateOutput {
                    model: candidate.model.clone(),
                    total_samples: Some(candidate.total_samples),
                    quality_sample: None,
                    cost_score: None,
                    tier_distance: None,
                    score: None,
                })
                .collect(),
        }
    }

    fn from_adaptive(choice: &AdaptiveChoice) -> Self {
        Self {
            phase: "adaptive".to_owned(),
            classified_tier: choice.classified_tier.clone(),
            request_type: choice.request_type.to_string(),
            eligible_mode: choice.eligible.as_str().to_owned(),
            quality_weight: format!("{:.6}", choice.weights.quality),
            cost_weight: format!("{:.6}", choice.weights.economy),
            tier_distance_penalty: format!("{:.6}", choice.tier_distance_penalty),
            chosen_model: choice.chosen_model.clone(),
            candidates: choice
                .candidates
                .iter()
                .map(|candidate| CandidateOutput {
                    model: candidate.model.clone(),
                    total_samples: None,
                    quality_sample: Some(format!("{:.6}", candidate.quality_sample)),
                    cost_score: Some(format!("{:.6}", candidate.cost_score)),
                    tier_distance: Some(candidate.tier_distance),
                    score: Some(format!("{:.6}", candidate.score)),
                })
                .collect(),
        }
    }
}
