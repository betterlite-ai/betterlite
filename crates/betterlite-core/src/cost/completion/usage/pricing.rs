use std::collections::BTreeMap;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage as PricingUsage, UsageNumber};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::api::responses_usage::{Usage as NormalizedUsage, UsageInput, normalize_usage};

use super::UsageConversionError;

// The logging usage as the JSON mapping the Responses normalizer reads back as typed counters.
fn usage_fields(usage: &Usage) -> Result<IndexMap<String, Json>, UsageConversionError> {
    let mut fields = IndexMap::new();
    for (name, value) in &usage.extras {
        fields.insert(name.clone(), value.clone());
    }
    fields.insert(
        "prompt_tokens".to_owned(),
        Json::Number(Number::from(usage.prompt_tokens)),
    );
    fields.insert(
        "completion_tokens".to_owned(),
        Json::Number(Number::from(usage.completion_tokens)),
    );
    fields.insert(
        "total_tokens".to_owned(),
        Json::Number(Number::from(usage.total_tokens)),
    );
    for (name, details) in [
        ("prompt_tokens_details", &usage.prompt_tokens_details),
        (
            "completion_tokens_details",
            &usage.completion_tokens_details,
        ),
        ("server_tool_use", &usage.server_tool_use),
    ] {
        if let Some(record) = details {
            fields.insert(name.to_owned(), Json::Object(record.clone()));
        }
    }
    if let Some(cost) = usage.cost {
        fields.insert(
            "cost".to_owned(),
            Json::Number(serde_json::from_str(&cost.to_string())?),
        );
    }
    Ok(fields)
}

// A float that no decimal holds is an error naming the extension; every other kind is skipped.
fn numeric_extras(
    converted: &NormalizedUsage,
) -> Result<BTreeMap<String, UsageNumber>, UsageConversionError> {
    let mut numeric_extras = BTreeMap::new();
    if let Some(cost) = converted.cost {
        numeric_extras.insert("cost".to_owned(), UsageNumber::Decimal(cost));
    }
    for (name, extension) in &converted.extensions {
        if name.starts_with('_') {
            continue;
        }
        if let Some(integer) = extension.as_i64() {
            numeric_extras.insert(name.clone(), UsageNumber::Integer(integer));
            continue;
        }
        if let Some(flag) = extension.as_bool() {
            numeric_extras.insert(name.clone(), UsageNumber::Boolean(flag));
            continue;
        }
        let Some(float) = extension.as_f64() else {
            continue;
        };
        let number = Decimal::try_from(float).map_err(|source| UsageConversionError::Decimal {
            name: name.clone(),
            source,
        })?;
        numeric_extras.insert(name.clone(), UsageNumber::Decimal(number));
    }
    Ok(numeric_extras)
}

// The logging usage the response carries, read as the typed block cost_per_token prices: details and extensions become typed counters.
pub fn pricing_usage(usage: &Usage) -> Result<PricingUsage, UsageConversionError> {
    let fields = usage_fields(usage)?;
    let mapping = serde_json::to_value(fields)?;
    let converted = normalize_usage(UsageInput::Mapping(&mapping))?;
    let extras = numeric_extras(&converted)?;
    Ok(PricingUsage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details: converted.prompt_tokens_details,
        completion_tokens_details: converted.completion_tokens_details,
        numeric_extras: extras,
    })
}
