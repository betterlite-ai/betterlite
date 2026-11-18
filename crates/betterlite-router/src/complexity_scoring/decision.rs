use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_redact::structured::mask_credentials_in_payload;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::complexity_scoring::config::TierBoundaries;
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::tier::Tier;

// StandardLoggingRoutingDecision: a JSON object whose optional facts are omitted rather than set to null
pub type RoutingDecision = IndexMap<String, Json>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingDecisionCause {
    PromptCacheCost,
    HeuristicScorer,
    HeuristicV2,
    ReasoningOverride,
    LlmClassifier,
    CapabilityClassifier,
    JevClassifier,
    LlmV2Classifier,
    LlmV2Fallback,
    HeuristicFirstShortCircuit,
    HybridShortCircuit,
    ClassifierPlugin,
    ClassifierFallback,
    CapabilityClassifierFallback,
    DefaultModelFallback,
    LiteralKeywordMatch,
    SemanticKeywordMatch,
    PlanMode,
    Housekeeping,
    ModalityEscalation,
    ModalityPinOverride,
    HealthFailover,
    HealthDefaultFallback,
    SessionAffinityPin,
    SessionAffinityEscalation,
    UserTurnContinuation,
    DefaultFallback,
    Keyword,
    QualityTier,
    Bandit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavingsBaseline<'input> {
    pub model: &'input str,
    pub deployment_id: Option<&'input str>,
}

pub struct DecisionInput<'input> {
    pub router_model_name: &'input str,
    pub routed_model: &'input str,
    pub cause: RoutingDecisionCause,
    pub conversation_continuing: bool,
    pub tier: Option<&'input Tier>,
    pub tier_label: Option<&'input str>,
    pub score: Option<f64>,
    pub boundaries: TierBoundaries,
    pub reasoning_override_min_score: f64,
    pub signals: &'input [String],
    pub matched_keyword: Option<&'input str>,
    pub escalation_keyword: Option<&'input str>,
    pub escalated: bool,
    pub classifier_model: Option<&'input str>,
    pub classifier_cost: Option<Decimal>,
    // the tier the decision had placed the request on before the context gate moved it
    pub context_original_tier: Option<&'input Tier>,
    pub tier_litellm_params: Option<&'input IndexMap<String, Json>>,
    pub savings_baseline: Option<SavingsBaseline<'input>>,
    pub previous_decision: Option<&'input RoutingDecision>,
}

pub fn build_routing_decision(input: &DecisionInput<'_>) -> Result<RoutingDecision, ScoringError> {
    let mut decision = RoutingDecision::new();
    // LiteLLM carries every classifier_* fact and the heuristic_v2 forecast from the decision being replaced
    if let Some(previous) = input.previous_decision {
        for (key, value) in previous {
            if key.starts_with("classifier_") || key == "heuristic_v2_forecast" {
                decision.insert(key.clone(), value.clone());
            }
        }
    }
    insert_base(&mut decision, input)?;
    insert_route(&mut decision, input)?;
    insert_evidence(&mut decision, input);
    insert_classifier_facts(&mut decision, input)?;
    Ok(decision)
}

fn insert_base(
    decision: &mut RoutingDecision,
    input: &DecisionInput<'_>,
) -> Result<(), ScoringError> {
    decision.insert(
        "router_model_name".to_owned(),
        Json::Text(input.router_model_name.to_owned()),
    );
    decision.insert(
        "router_type".to_owned(),
        Json::Text("complexity".to_owned()),
    );
    decision.insert(
        "routed_model".to_owned(),
        Json::Text(input.routed_model.to_owned()),
    );
    let cause = serde_json::to_value(input.cause)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| ScoringError::Record { source })?;
    decision.insert("cause".to_owned(), cause);
    decision.insert(
        "conversation_continuing".to_owned(),
        Json::Bool(input.conversation_continuing),
    );
    if let Some(baseline) = input.savings_baseline {
        decision.insert(
            "savings_baseline_model".to_owned(),
            Json::Text(baseline.model.to_owned()),
        );
        if let Some(deployment) = baseline.deployment_id {
            decision.insert(
                "savings_baseline_deployment_id".to_owned(),
                Json::Text(deployment.to_owned()),
            );
        }
    }
    Ok(())
}

// the tier and score travel with the boundaries and floor that mapped the score, which alone give them meaning
fn insert_route(
    decision: &mut RoutingDecision,
    input: &DecisionInput<'_>,
) -> Result<(), ScoringError> {
    if let Some(tier) = input.tier {
        let name = tier.name();
        decision.insert("tier".to_owned(), Json::Text(name.to_owned()));
        if let Some(label) = input.tier_label
            && label != name
        {
            decision.insert("tier_label".to_owned(), Json::Text(label.to_owned()));
        }
    }
    if let Some(score) = input.score {
        decision.insert("score".to_owned(), number(score, "score")?);
        decision.insert(
            "tier_boundaries".to_owned(),
            boundaries_json(input.boundaries)?,
        );
        decision.insert(
            "reasoning_override_min_score".to_owned(),
            number(
                input.reasoning_override_min_score,
                "reasoning_override_min_score",
            )?,
        );
    }
    Ok(())
}

fn insert_evidence(decision: &mut RoutingDecision, input: &DecisionInput<'_>) {
    if !input.signals.is_empty() {
        let signals = input
            .signals
            .iter()
            .map(|signal| Json::Text(signal.clone()))
            .collect();
        decision.insert("signals".to_owned(), Json::List(signals));
    }
    if let Some(keyword) = input.matched_keyword {
        decision.insert("matched_keyword".to_owned(), Json::Text(keyword.to_owned()));
    }
    if let Some(keyword) = input.escalation_keyword {
        // the caller asked to escalate and whether the tier moved; the top tier records escalated=false
        decision.insert(
            "escalation_keyword".to_owned(),
            Json::Text(keyword.to_owned()),
        );
        decision.insert("escalated".to_owned(), Json::Bool(input.escalated));
    }
}

fn insert_classifier_facts(
    decision: &mut RoutingDecision,
    input: &DecisionInput<'_>,
) -> Result<(), ScoringError> {
    let mut facts = Vec::with_capacity(4);
    if let Some(model) = input.classifier_model {
        facts.push(("classifier_model", Json::Text(model.to_owned())));
    }
    if let Some(cost) = input.classifier_cost {
        facts.push(("classifier_cost", classifier_cost(cost)?));
    }
    if let Some(original) = input.context_original_tier {
        // the gate moved the request off its decided tier, and the original tier names where it was
        facts.push(("context_escalated", Json::Bool(true)));
        facts.push((
            "context_escalation_original_tier",
            Json::Text(original.name().to_owned()),
        ));
    }
    if let Some(params) = input.tier_litellm_params
        && let Some(masked) = masked_params(params)
    {
        facts.push(("tier_litellm_params", masked));
    }
    for (key, value) in facts {
        decision.insert(key.to_owned(), value);
    }
    Ok(())
}

// LiteLLM records the cost as a float, so a whole number keeps the decimal point its float carries
fn classifier_cost(value: Decimal) -> Result<Json, ScoringError> {
    let normalized = value.normalize().to_string();
    let written = if normalized.contains('.') {
        normalized
    } else {
        format!("{normalized}.0")
    };
    Number::from_str(&written)
        .map(Json::Number)
        .map_err(|source| ScoringError::Record { source })
}

fn number(value: f64, field: &str) -> Result<Json, ScoringError> {
    Number::from_f64(value)
        .map(Json::Number)
        .ok_or_else(|| ScoringError::NonFinite {
            field: field.to_owned(),
        })
}

fn boundaries_json(boundaries: TierBoundaries) -> Result<Json, ScoringError> {
    let mut fields = IndexMap::with_capacity(3);
    fields.insert(
        "simple_medium".to_owned(),
        number(boundaries.simple_medium, "tier_boundaries.simple_medium")?,
    );
    fields.insert(
        "medium_complex".to_owned(),
        number(boundaries.medium_complex, "tier_boundaries.medium_complex")?,
    );
    fields.insert(
        "complex_reasoning".to_owned(),
        number(
            boundaries.complex_reasoning,
            "tier_boundaries.complex_reasoning",
        )?,
    );
    Ok(Json::Object(fields))
}

// the masker returns a bare string past its work limit, and LiteLLM stores the key only for a mapping
fn masked_params(params: &IndexMap<String, Json>) -> Option<Json> {
    if params.is_empty() {
        return None;
    }
    let value = serde_json::to_value(params).ok()?;
    let masked = mask_credentials_in_payload(&value);
    match serde_json::from_value(masked).ok()? {
        Json::Object(fields) => Some(Json::Object(fields)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
