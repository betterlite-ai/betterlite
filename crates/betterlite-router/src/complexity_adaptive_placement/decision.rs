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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColdStartCandidate {
    pub model: String,
    pub total_samples: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateScore {
    pub model: String,
    pub quality_sample: f64,
    pub economy_score: f64,
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

// the fields both phases record apart from the phase, chosen model and candidates
struct DecisionFields<'decision> {
    classified_tier: &'decision str,
    request_type: RequestType,
    eligible_mode: &'static str,
    weights: AdaptiveRouterWeights,
    tier_distance_penalty: f64,
    chosen_model: &'decision str,
}

impl AdaptiveDecision {
    fn to_json(&self) -> Result<Json, AdaptivePlacementError> {
        let (phase, candidates) = match self {
            Self::ColdStart(decision) => (
                "cold_start",
                cold_start_candidates_json(&decision.candidates),
            ),
            Self::Adaptive(choice) => ("adaptive", candidate_scores_json(&choice.candidates)?),
        };
        render(&decision_fields(self), phase, candidates)
    }
}

fn decision_fields(decision: &AdaptiveDecision) -> DecisionFields<'_> {
    match decision {
        AdaptiveDecision::ColdStart(decision) => DecisionFields {
            classified_tier: &decision.classified_tier,
            request_type: decision.request_type,
            eligible_mode: "classified_tier",
            weights: decision.weights,
            tier_distance_penalty: decision.tier_distance_penalty,
            chosen_model: &decision.chosen_model,
        },
        AdaptiveDecision::Adaptive(choice) => DecisionFields {
            classified_tier: &choice.classified_tier,
            request_type: choice.request_type,
            eligible_mode: choice.eligible.as_str(),
            weights: choice.weights,
            tier_distance_penalty: choice.tier_distance_penalty,
            chosen_model: &choice.chosen_model,
        },
    }
}

// the source's metadata entry, in the order _soft_floor_pick fills it
fn render(
    fields: &DecisionFields<'_>,
    phase: &str,
    candidates: Vec<Json>,
) -> Result<Json, AdaptivePlacementError> {
    let mut object = IndexMap::new();
    object.insert(
        "classified_tier".to_owned(),
        Json::Text(fields.classified_tier.to_owned()),
    );
    object.insert(
        "request_type".to_owned(),
        Json::Text(fields.request_type.to_string()),
    );
    object.insert(
        "eligible_mode".to_owned(),
        Json::Text(fields.eligible_mode.to_owned()),
    );
    object.insert(
        "quality_weight".to_owned(),
        number(fields.weights.quality, "quality_weight")?,
    );
    object.insert(
        "cost_weight".to_owned(),
        number(fields.weights.economy, "cost_weight")?,
    );
    object.insert(
        "tier_distance_penalty".to_owned(),
        number(fields.tier_distance_penalty, "tier_distance_penalty")?,
    );
    object.insert("phase".to_owned(), Json::Text(phase.to_owned()));
    object.insert(
        "chosen_model".to_owned(),
        Json::Text(fields.chosen_model.to_owned()),
    );
    object.insert("candidates".to_owned(), Json::List(candidates));
    Ok(Json::Object(object))
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
            number(candidate.economy_score, "cost_score")?,
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
