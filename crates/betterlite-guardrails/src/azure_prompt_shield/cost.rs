use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_env::number;
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
    let Some(raw) = raw else {
        return Ok(None);
    };
    let resolved = resolved_secret_value(raw, lookup)?;
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
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Text(text) => price_from_text(text),
        GuardrailValue::SignedInteger(amount) => Ok(Some(Decimal::from(*amount))),
        GuardrailValue::UnsignedInteger(amount) => Ok(Some(Decimal::from(*amount))),
        GuardrailValue::Decimal(amount) => finite_price(*amount),
        GuardrailValue::Boolean(_) | GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            Err(PromptShieldError::PriceNotANumber {
                value: describe_value(value),
            })
        }
    }
}

fn price_from_text(text: &str) -> Result<Option<Decimal>, PromptShieldError> {
    match number::<f64>(PRICE_KEY, text) {
        Ok(Some(amount)) => finite_price(amount),
        Ok(None) => Ok(None),
        Err(_source) => Err(PromptShieldError::PriceNotANumber {
            value: format!("{text:?}"),
        }),
    }
}

fn finite_price(amount: f64) -> Result<Option<Decimal>, PromptShieldError> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(PromptShieldError::InvalidPrice {
            value: describe_value(&GuardrailValue::Decimal(amount)),
        });
    }
    Decimal::try_from(amount)
        .map(Some)
        .map_err(|_source| PromptShieldError::InvalidPrice {
            value: describe_value(&GuardrailValue::Decimal(amount)),
        })
}
