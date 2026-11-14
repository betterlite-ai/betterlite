use std::collections::BTreeMap;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::usage_sum::{Usage as PricingUsage, UsageNumber};
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use crate::api::responses_usage::{Usage as NormalizedUsage, UsageInput, normalize_usage};

use super::UsageConversionError;

// The logging usage as the JSON mapping the Responses normalizer reads back as typed counters.
fn usage_fields(usage: &Usage) -> Result<Value, UsageConversionError> {
    let mut fields = Map::new();
    for (name, value) in &usage.extras {
        fields.insert(name.clone(), serde_json::to_value(value)?);
    }
    fields.insert("prompt_tokens".to_owned(), Value::from(usage.prompt_tokens));
    fields.insert(
        "completion_tokens".to_owned(),
        Value::from(usage.completion_tokens),
    );
    fields.insert("total_tokens".to_owned(), Value::from(usage.total_tokens));
    for (name, details) in [
        ("prompt_tokens_details", &usage.prompt_tokens_details),
        (
            "completion_tokens_details",
            &usage.completion_tokens_details,
        ),
        ("server_tool_use", &usage.server_tool_use),
    ] {
        if let Some(record) = details {
            fields.insert(name.to_owned(), serde_json::to_value(record)?);
        }
    }
    if let Some(cost) = usage.cost {
        fields.insert("cost".to_owned(), serde_json::from_str(&cost.to_string())?);
    }
    Ok(Value::Object(fields))
}

// One extension keeps the kind of its JSON value; a float that no decimal holds is an error naming the extension.
fn extension_number(
    name: &str,
    extension: &Value,
) -> Result<Option<UsageNumber>, UsageConversionError> {
    if let Some(integer) = extension.as_i64() {
        return Ok(Some(UsageNumber::Integer(integer)));
    }
    if let Some(flag) = extension.as_bool() {
        return Ok(Some(UsageNumber::Boolean(flag)));
    }
    let Some(float) = extension.as_f64() else {
        return Ok(None);
    };
    let number = Decimal::try_from(float).map_err(|source| UsageConversionError::Decimal {
        name: name.to_owned(),
        source,
    })?;
    Ok(Some(UsageNumber::Decimal(number)))
}

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
        if let Some(number) = extension_number(name, extension)? {
            numeric_extras.insert(name.clone(), number);
        }
    }
    Ok(numeric_extras)
}

// The logging usage the response carries, read as the typed block cost_per_token prices: details and extensions become typed counters.
pub fn pricing_usage(usage: &Usage) -> Result<PricingUsage, UsageConversionError> {
    let fields = usage_fields(usage)?;
    let converted = normalize_usage(UsageInput::Mapping(&fields))?;
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
