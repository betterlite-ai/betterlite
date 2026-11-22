use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_router::adaptive_classification::request_type::RequestType;
use serde::{Deserialize, Serialize};

use crate::router_probes::error::OperationError;

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
    pub(super) preferences: BTreeMap<String, PreferenceOutput>,
}

// _ensure_adaptive_router's model_to_prefs, keyed by model so the pooled order never matters
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PreferenceOutput {
    pub(super) quality_tier: u8,
    pub(super) strengths: Vec<String>,
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

// the object the router rendered into request metadata, which is what the Python probe reads back
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    phase: String,
    classified_tier: String,
    request_type: String,
    eligible_mode: String,
    quality_weight: f64,
    cost_weight: f64,
    tier_distance_penalty: f64,
    chosen_model: String,
    candidates: Vec<RawCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidate {
    model: String,
    total_samples: Option<u64>,
    quality_sample: Option<f64>,
    cost_score: Option<f64>,
    tier_distance: Option<u32>,
    score: Option<f64>,
}

impl DecisionOutput {
    pub(super) fn from_json(value: &Json) -> Result<Self, OperationError> {
        let text =
            serde_json::to_string(value).map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        let raw: RawDecision =
            serde_json::from_str(&text).map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        let RawDecision {
            phase,
            classified_tier,
            request_type,
            eligible_mode,
            quality_weight,
            cost_weight,
            tier_distance_penalty,
            chosen_model,
            candidates,
        } = raw;
        Ok(Self {
            phase,
            classified_tier,
            request_type,
            eligible_mode,
            quality_weight: format!("{quality_weight:.6}"),
            cost_weight: format!("{cost_weight:.6}"),
            tier_distance_penalty: format!("{tier_distance_penalty:.6}"),
            chosen_model,
            candidates: candidates
                .into_iter()
                .map(CandidateOutput::from_raw)
                .collect(),
        })
    }
}

impl CandidateOutput {
    fn from_raw(raw: RawCandidate) -> Self {
        let RawCandidate {
            model,
            total_samples,
            quality_sample,
            cost_score,
            tier_distance,
            score,
        } = raw;
        Self {
            model,
            total_samples,
            quality_sample: quality_sample.map(|value| format!("{value:.6}")),
            cost_score: cost_score.map(|value| format!("{value:.6}")),
            tier_distance,
            score: score.map(|value| format!("{value:.6}")),
        }
    }
}
