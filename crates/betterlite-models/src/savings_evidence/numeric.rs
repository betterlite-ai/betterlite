use rust_decimal::Decimal;
use serde::Deserialize;

use super::metadata::Metadata;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RoutingDecision {
    pub classifier_cost: Option<Scalar>,
}

// A bool is a JSON Bool, never a Number, so it is excluded by the match alone.
#[must_use]
pub fn numeric_savings(value: Option<&Scalar>) -> Option<Decimal> {
    match value {
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        _ => None,
    }
}

#[must_use]
pub fn recorded_autorouter_savings(metadata: &Metadata) -> Option<Decimal> {
    let estimate = metadata
        .autorouter_savings_estimate
        .as_ref()?
        .as_mapping()?;
    if !is_estimated_and_versioned(estimate) {
        return None;
    }
    numeric_savings(metadata.autorouter_savings.as_ref())
}

// The lexeme's raw text, not its mathematical value, decides "exact integer": "2" and "2.0" stay distinct.
fn is_estimated_and_versioned(estimate: &super::metadata::AutorouterEstimate) -> bool {
    let exact_version = matches!(
        &estimate.version,
        Some(Scalar::Number(number)) if matches!(number.as_i64(), Some(1..=3))
    );
    exact_version && matches!(&estimate.status, Some(Scalar::Text(status)) if status == "estimated")
}

#[must_use]
pub fn classifier_cost_from_decision(
    routing_decision: Option<&RoutingDecision>,
) -> Option<Decimal> {
    let classifier_cost = routing_decision.and_then(|decision| decision.classifier_cost.as_ref());
    numeric_savings(classifier_cost)
}

mod tests;
