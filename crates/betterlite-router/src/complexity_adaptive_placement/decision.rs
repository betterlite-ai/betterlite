use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::adaptive_classification::config::AdaptiveRouterWeights;
use crate::adaptive_classification::request_type::RequestType;
use crate::router_predicates::JsonMap;

use super::error::AdaptivePlacementError;

const DECISION_KEY: &str = "adaptive_router_decision";
const METADATA_KEY: &str = "metadata";

#[derive(Clone, Debug, PartialEq)]
pub struct ColdStartCandidate {
    pub model: String,
    pub total_samples: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateScore {
    pub model: String,
    pub quality_sample: f64,
    pub cost_score: f64,
    pub tier_distance: u32,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColdStartDecision {
    pub classified_tier: String,
    pub request_type: RequestType,
    pub weights: AdaptiveRouterWeights,
    pub tier_distance_penalty: f64,
    pub chosen_model: String,
    pub candidates: Vec<ColdStartCandidate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdaptiveChoice {
    pub classified_tier: String,
    pub request_type: RequestType,
    pub eligible: AdaptiveEligible,
    pub weights: AdaptiveRouterWeights,
    pub tier_distance_penalty: f64,
    pub chosen_model: String,
    pub candidates: Vec<CandidateScore>,
}

// the typed adaptive_router_decision metadata _soft_floor_pick records on the request
#[derive(Clone, Debug, PartialEq)]
pub enum AdaptiveDecision {
    ColdStart(ColdStartDecision),
    Adaptive(AdaptiveChoice),
}

impl AdaptiveDecision {
    fn to_json(&self) -> Result<Json, AdaptivePlacementError> {
        match self {
            Self::ColdStart(decision) => decision.to_json(),
            Self::Adaptive(choice) => choice.to_json(),
        }
    }
}

impl ColdStartDecision {
    fn to_json(&self) -> Result<Json, AdaptivePlacementError> {
        let mut fields = shared_fields(
            &self.classified_tier,
            self.request_type,
            "classified_tier",
            self.weights,
            self.tier_distance_penalty,
        )?;
        fields.insert("phase".to_owned(), Json::Text("cold_start".to_owned()));
        fields.insert(
            "chosen_model".to_owned(),
            Json::Text(self.chosen_model.clone()),
        );
        fields.insert(
            "candidates".to_owned(),
            Json::List(cold_start_candidates_json(&self.candidates)),
        );
        Ok(Json::Object(fields))
    }
}

impl AdaptiveChoice {
    fn to_json(&self) -> Result<Json, AdaptivePlacementError> {
        let mut fields = shared_fields(
            &self.classified_tier,
            self.request_type,
            self.eligible.as_str(),
            self.weights,
            self.tier_distance_penalty,
        )?;
        fields.insert("phase".to_owned(), Json::Text("adaptive".to_owned()));
        fields.insert(
            "chosen_model".to_owned(),
            Json::Text(self.chosen_model.clone()),
        );
        fields.insert(
            "candidates".to_owned(),
            Json::List(candidate_scores_json(&self.candidates)?),
        );
        Ok(Json::Object(fields))
    }
}

// the keys both decision phases record, in the order the source writes them
fn shared_fields(
    classified_tier: &str,
    request_type: RequestType,
    eligible_mode: &str,
    weights: AdaptiveRouterWeights,
    tier_distance_penalty: f64,
) -> Result<IndexMap<String, Json>, AdaptivePlacementError> {
    let mut fields = IndexMap::new();
    fields.insert(
        "classified_tier".to_owned(),
        Json::Text(classified_tier.to_owned()),
    );
    fields.insert(
        "request_type".to_owned(),
        Json::Text(request_type.to_string()),
    );
    fields.insert(
        "eligible_mode".to_owned(),
        Json::Text(eligible_mode.to_owned()),
    );
    fields.insert(
        "quality_weight".to_owned(),
        number(weights.quality, "quality_weight")?,
    );
    fields.insert(
        "cost_weight".to_owned(),
        number(weights.economy, "cost_weight")?,
    );
    fields.insert(
        "tier_distance_penalty".to_owned(),
        number(tier_distance_penalty, "tier_distance_penalty")?,
    );
    Ok(fields)
}

fn cold_start_candidates_json(candidates: &[ColdStartCandidate]) -> Vec<Json> {
    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let mut fields = IndexMap::new();
        fields.insert("model".to_owned(), Json::Text(candidate.model.clone()));
        fields.insert(
            "total_samples".to_owned(),
            Json::Number(Number::from(candidate.total_samples)),
        );
        entries.push(Json::Object(fields));
    }
    entries
}

fn candidate_scores_json(
    candidates: &[CandidateScore],
) -> Result<Vec<Json>, AdaptivePlacementError> {
    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let mut fields = IndexMap::new();
        fields.insert("model".to_owned(), Json::Text(candidate.model.clone()));
        fields.insert(
            "quality_sample".to_owned(),
            number(candidate.quality_sample, "quality_sample")?,
        );
        fields.insert(
            "cost_score".to_owned(),
            number(candidate.cost_score, "cost_score")?,
        );
        fields.insert(
            "tier_distance".to_owned(),
            Json::Number(Number::from(candidate.tier_distance)),
        );
        fields.insert("score".to_owned(), number(candidate.score, "score")?);
        entries.push(Json::Object(fields));
    }
    Ok(entries)
}

fn number(value: f64, field: &'static str) -> Result<Json, AdaptivePlacementError> {
    Number::from_f64(value)
        .map(Json::Number)
        .ok_or(AdaptivePlacementError::NonFinite { field })
}

// request_kwargs.setdefault("metadata", {}) then the isinstance(metadata, dict) guard
pub fn write_decision(
    request_kwargs: Option<&mut JsonMap>,
    decision: &AdaptiveDecision,
) -> Result<(), AdaptivePlacementError> {
    let Some(kwargs) = request_kwargs else {
        return Ok(());
    };
    let entry = kwargs
        .entry(METADATA_KEY.to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(fields) = entry else {
        return Ok(());
    };
    fields.insert(DECISION_KEY.to_owned(), decision.to_json()?);
    Ok(())
}
