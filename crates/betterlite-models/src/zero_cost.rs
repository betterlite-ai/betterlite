use indexmap::IndexMap;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use crate::decisions::Json;
mod tests;

pub const ZERO_COST_COUNTER_NAME: &str = "litellm_zero_cost_requests_total";
pub const TEXT_INPUT_RATE: &str = "input_cost_per_token";
pub const AUDIO_INPUT_RATE: &str = "input_cost_per_audio_token";
pub const TEXT_OUTPUT_RATE: &str = "output_cost_per_token";
pub const AUDIO_OUTPUT_RATE: &str = "output_cost_per_audio_token";
pub const RATE_KEY_MARKERS: [&str; 2] = ["cost", "pricing"];
pub const MAX_PRICING_DEPTH: u32 = 4;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroCostReason {
    MissingPricingKey,
    PricingNotApplied,
    CostCalculationError,
}
impl ZeroCostReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MissingPricingKey => "missing_pricing_key",
            Self::PricingNotApplied => "pricing_not_applied",
            Self::CostCalculationError => "cost_calculation_error",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroCostDiagnostic {
    pub reason: ZeroCostReason,
    pub pricing_model: String,
    pub missing_pricing_keys: Vec<&'static str>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub prompt_audio_tokens: i64,
    pub completion_audio_tokens: i64,
}

// The four source rate names stay in this order, so the missing-key list keeps it too.
#[must_use]
pub fn used_pricing_keys(usage: &TokenUsage) -> Vec<&'static str> {
    let prompt_audio = usage.prompt_audio_tokens.max(0);
    let completion_audio = usage.completion_audio_tokens.max(0);
    [
        (
            TEXT_INPUT_RATE,
            usage.prompt_tokens.max(0).saturating_sub(prompt_audio),
        ),
        (AUDIO_INPUT_RATE, prompt_audio),
        (
            TEXT_OUTPUT_RATE,
            usage
                .completion_tokens
                .max(0)
                .saturating_sub(completion_audio),
        ),
        (AUDIO_OUTPUT_RATE, completion_audio),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(key, _)| key)
    .collect()
}
#[must_use]
pub fn diagnose_zero_cost(
    usage: &TokenUsage,
    pricing_model: &str,
    pricing_entry: &IndexMap<String, Json>,
    calculation_failed: bool,
) -> Option<ZeroCostDiagnostic> {
    let used_keys = used_pricing_keys(usage);
    if used_keys.is_empty() {
        return None;
    }
    let missing_keys = missing_rates(&used_keys, pricing_entry);
    let every_used_rate_is_zero = used_keys.iter().all(|key| {
        pricing_entry
            .get(*key)
            .is_some_and(|value| number_amount(value).is_some_and(|amount| amount.is_zero()))
    });
    if missing_keys.is_empty() && every_used_rate_is_zero {
        return None;
    }
    if !declares_a_rate(pricing_entry) {
        return None;
    }
    let reason = if calculation_failed {
        ZeroCostReason::CostCalculationError
    } else if missing_keys.is_empty() {
        ZeroCostReason::PricingNotApplied
    } else {
        ZeroCostReason::MissingPricingKey
    };
    let keys = if reason == ZeroCostReason::MissingPricingKey {
        missing_keys
    } else {
        Vec::new()
    };
    Some(ZeroCostDiagnostic {
        reason,
        pricing_model: pricing_model.to_owned(),
        missing_pricing_keys: keys,
    })
}
fn missing_rates(
    used_keys: &[&'static str],
    pricing_entry: &IndexMap<String, Json>,
) -> Vec<&'static str> {
    used_keys
        .iter()
        .copied()
        .filter(|key| {
            pricing_entry
                .get(*key)
                .is_none_or(|value| matches!(value, Json::Null))
        })
        .collect()
}
#[must_use]
pub fn zero_cost_warning(
    diagnostic: &ZeroCostDiagnostic,
    model_group: Option<&str>,
    model: &str,
    custom_llm_provider: Option<&str>,
    usage: &TokenUsage,
) -> String {
    format!(
        "Billable request priced at $0 and logged as such (model_group={} model={model} provider={} prompt_tokens={} completion_tokens={}): {}. Counted in {ZERO_COST_COUNTER_NAME}{{reason=\"{}\"}}",
        model_group.unwrap_or(model),
        custom_llm_provider.unwrap_or("unknown"),
        usage.prompt_tokens.max(0),
        usage.completion_tokens.max(0),
        cause(diagnostic),
        diagnostic.reason.label()
    )
}
fn cause(diagnostic: &ZeroCostDiagnostic) -> String {
    match diagnostic.reason {
        ZeroCostReason::MissingPricingKey => format!(
            "pricing entry '{}' has no {}. Set the missing rate in the deployment's model_info or in the model cost map, or set every rate to 0 to mark the model free",
            diagnostic.pricing_model,
            diagnostic.missing_pricing_keys.join(", ")
        ),
        ZeroCostReason::PricingNotApplied => format!(
            "pricing entry '{}' declares non-zero rates for this usage, but the cost calculator returned $0",
            diagnostic.pricing_model
        ),
        ZeroCostReason::CostCalculationError => format!(
            "cost calculation raised for pricing entry '{}', see response_cost_failure_debug_information",
            diagnostic.pricing_model
        ),
    }
}
fn rate_values(value: &Json) -> Vec<&Json> {
    match value {
        Json::Object(fields) => rate_children(fields),
        Json::List(items) => items.iter().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => vec![value],
    }
}
fn rate_children(fields: &IndexMap<String, Json>) -> Vec<&Json> {
    fields
        .iter()
        .filter(|(name, _)| RATE_KEY_MARKERS.iter().any(|marker| name.contains(marker)))
        .map(|(_, value)| value)
        .collect()
}
fn declares_a_rate(pricing_entry: &IndexMap<String, Json>) -> bool {
    let mut values = rate_children(pricing_entry);
    for _ in 1..MAX_PRICING_DEPTH {
        values = values.iter().flat_map(|value| rate_values(value)).collect();
    }
    values
        .iter()
        .any(|value| number_amount(value).is_some_and(|amount| amount > Decimal::ZERO))
}
fn number_amount(value: &Json) -> Option<Decimal> {
    let Json::Number(number) = value else {
        return None;
    };
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        Decimal::from_str(&text)
    };
    parsed.ok()
}
