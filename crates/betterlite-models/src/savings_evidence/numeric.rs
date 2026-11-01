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

// The lexeme's raw text, not its mathematical value, decides "exact integer": "2" and "2.0" stay distinct.
#[must_use]
pub fn recorded_autorouter_savings(metadata: &Metadata) -> Option<Decimal> {
    let estimate = metadata
        .autorouter_savings_estimate
        .as_ref()?
        .as_mapping()?;
    let exact_version = matches!(
        &estimate.version,
        Some(Scalar::Number(number)) if matches!(number.as_i64(), Some(1..=3))
    );
    let is_estimated =
        matches!(&estimate.status, Some(Scalar::Text(status)) if status == "estimated");
    if !exact_version || !is_estimated {
        return None;
    }
    numeric_savings(metadata.autorouter_savings.as_ref())
}

mod tests;
