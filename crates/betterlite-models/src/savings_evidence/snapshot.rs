use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use super::error::SavingsEvidenceError;
use super::pricing_basis::PricingBasis;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

// Every field mirrors one key read from a request's logged cost_breakdown mapping; an unknown key is ignored.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CostBreakdown {
    pub service_tier: Option<Scalar>,
    pub data_residency: Option<Scalar>,
    pub vertex_location: Option<Scalar>,
    pub input_cost: Option<Scalar>,
    pub output_cost: Option<Scalar>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "RawBaselineCostSnapshot")]
pub struct BaselineCostSnapshot {
    pub model: String,
    pub provider: String,
    pub prices: Option<ModelInfo>,
    pub basis: PricingBasis,
    pub actual_spend: Decimal,
    pub actual_token_cost: Option<Decimal>,
    pub classifier_cost: Decimal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBaselineCostSnapshot {
    model: String,
    provider: String,
    #[serde(default)]
    prices: Option<ModelInfo>,
    #[serde(default)]
    basis: PricingBasis,
    actual_spend: StrictDecimal,
    #[serde(default)]
    actual_token_cost: Option<StrictDecimal>,
    #[serde(default)]
    classifier_cost: Option<StrictDecimal>,
}

// A separate newtype only accepts serde_json::Number: rust_decimal's own impl also accepts a JSON string.
#[derive(Deserialize)]
#[serde(try_from = "Number")]
struct StrictDecimal(Decimal);

impl TryFrom<Number> for StrictDecimal {
    type Error = SavingsEvidenceError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        parse_decimal_lexeme(&number.to_string())
            .map(Self)
            .map_err(|_source| SavingsEvidenceError::NonNumericAmount)
    }
}

impl TryFrom<RawBaselineCostSnapshot> for BaselineCostSnapshot {
    type Error = SavingsEvidenceError;

    fn try_from(raw: RawBaselineCostSnapshot) -> Result<Self, Self::Error> {
        let actual_spend = non_negative(raw.actual_spend.0, "actual_spend")?;
        let actual_token_cost = match raw.actual_token_cost {
            Some(value) => Some(non_negative(value.0, "actual_token_cost")?),
            None => None,
        };
        let classifier_cost = match raw.classifier_cost {
            Some(value) => non_negative(value.0, "classifier_cost")?,
            None => Decimal::ZERO,
        };
        Ok(Self {
            model: raw.model,
            provider: raw.provider,
            prices: raw.prices,
            basis: raw.basis,
            actual_spend,
            actual_token_cost,
            classifier_cost,
        })
    }
}

const fn non_negative(
    amount: Decimal,
    field: &'static str,
) -> Result<Decimal, SavingsEvidenceError> {
    if amount.is_sign_negative() {
        return Err(SavingsEvidenceError::NegativeAmount { field });
    }
    Ok(amount)
}

// Nothing here is guaranteed to be a string; a nonstring value is dropped rather than coerced or inferred.
#[must_use]
pub fn pricing_basis_from_breakdown(cost_breakdown: Option<&CostBreakdown>) -> PricingBasis {
    let Some(breakdown) = cost_breakdown else {
        return PricingBasis::default();
    };
    PricingBasis {
        service_tier: string_field(breakdown.service_tier.as_ref()),
        data_residency: string_field(breakdown.data_residency.as_ref()),
        vertex_location: string_field(breakdown.vertex_location.as_ref()),
    }
}

// input_cost and output_cost already sum to the full recorded token cost; the cache components must never add again.
#[must_use]
pub fn recorded_token_cost(cost_breakdown: Option<&CostBreakdown>) -> Option<Decimal> {
    let breakdown = cost_breakdown?;
    let input_cost = numeric_cost_component(breakdown.input_cost.as_ref())?;
    let output_cost = numeric_cost_component(breakdown.output_cost.as_ref())?;
    input_cost.checked_add(output_cost)
}

fn numeric_cost_component(value: Option<&Scalar>) -> Option<Decimal> {
    match value {
        Some(Scalar::Bool(flag)) => Some(Decimal::from(i64::from(*flag))),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        _ => None,
    }
}

fn string_field(value: Option<&Scalar>) -> Option<String> {
    match value {
        Some(Scalar::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

mod tests;
