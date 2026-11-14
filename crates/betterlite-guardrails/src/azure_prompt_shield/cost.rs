use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rust_decimal::Decimal;

use super::error::{PromptShieldError, describe_value};
use super::secret::resolved_secret_value;
use crate::hook_initializers::secret_lookup::SecretLookup;

pub const COST_TIER_KEY: &str = "cost_tier";
pub const PRICE_KEY: &str = "price_per_1000_text_records";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostTier {
    Free,
    Paid,
}

impl CostTier {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Paid => "paid",
        }
    }
}

// LiteLLM's _resolved_cost_tier: unset or blank means no tier, anything else is stripped and lowercased.
pub fn resolve_cost_tier(
    raw: Option<&GuardrailValue>,
    lookup: &dyn SecretLookup,
) -> Result<Option<CostTier>, PromptShieldError> {
    let Some(configured) = raw else {
        return Ok(None);
    };
    let resolved = resolved_secret_value(configured, lookup)?;
    match &resolved {
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Text(text) if text.trim().is_empty() => Ok(None),
        GuardrailValue::Text(text) => match text.trim().to_lowercase().as_str() {
            "free" => Ok(Some(CostTier::Free)),
            "paid" => Ok(Some(CostTier::Paid)),
            _other => Err(PromptShieldError::InvalidCostTier {
                value: describe_value(&resolved),
            }),
        },
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => Err(PromptShieldError::InvalidCostTier {
            value: describe_value(&resolved),
        }),
    }
}

// LiteLLM's _resolved_price: a paid tier without a positive price fails the deployment at startup.
pub fn resolve_price(
    raw: Option<&GuardrailValue>,
    cost_tier: Option<CostTier>,
    lookup: &dyn SecretLookup,
) -> Result<Option<Decimal>, PromptShieldError> {
    let resolved = raw
        .map(|value| resolved_secret_value(value, lookup))
        .transpose()?;
    let price = price_from_value(resolved.as_ref())?;
    if cost_tier == Some(CostTier::Paid) && price.is_none_or(|amount| amount <= Decimal::ZERO) {
        return Err(PromptShieldError::PaidTierWithoutPrice);
    }
    Ok(price)
}

// LiteLLM's _price_from_value: a bool or a container is not a number, and a blank value means no price.
pub fn price_from_value(
    value: Option<&GuardrailValue>,
) -> Result<Option<Decimal>, PromptShieldError> {
    let Some(configured) = value else {
        return Ok(None);
    };
    match configured {
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Text(text) => price_from_text(text),
        GuardrailValue::SignedInteger(amount) => checked_price(Decimal::from(*amount), configured),
        GuardrailValue::UnsignedInteger(amount) => {
            checked_price(Decimal::from(*amount), configured)
        }
        GuardrailValue::Decimal(amount) => float_price(*amount),
        GuardrailValue::Boolean(_) | GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            Err(PromptShieldError::PriceNotANumber {
                value: describe_value(configured),
                source: None,
            })
        }
    }
}

// LiteLLM's _price_from_value for text: float(value), then the same finite, non-negative check.
fn price_from_text(text: &str) -> Result<Option<Decimal>, PromptShieldError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let amount =
        trimmed
            .parse::<Decimal>()
            .map_err(|source| PromptShieldError::PriceNotANumber {
                value: format!("{text:?}"),
                source: Some(source),
            })?;
    checked_price(amount, &GuardrailValue::Text(text.to_owned()))
}

// LiteLLM's math.isfinite and price < 0 checks for a value the config already delivered as a float.
fn float_price(amount: f64) -> Result<Option<Decimal>, PromptShieldError> {
    if !amount.is_finite() || amount < 0.0_f64 {
        return Err(PromptShieldError::InvalidPrice {
            value: describe_value(&GuardrailValue::Decimal(amount)),
            source: None,
        });
    }
    let decimal = Decimal::try_from(amount).map_err(|source| PromptShieldError::InvalidPrice {
        value: describe_value(&GuardrailValue::Decimal(amount)),
        source: Some(source),
    })?;
    Ok(Some(decimal))
}

// Negative integers take the same path as negative floats in LiteLLM: one finite, non-negative check.
fn checked_price(
    amount: Decimal,
    value: &GuardrailValue,
) -> Result<Option<Decimal>, PromptShieldError> {
    if amount < Decimal::ZERO {
        return Err(PromptShieldError::InvalidPrice {
            value: describe_value(value),
            source: None,
        });
    }
    Ok(Some(amount))
}
