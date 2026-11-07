use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::usage::{Usage, UsageError};

pub fn usage_from_mapping(mapping: &IndexMap<String, Json>) -> Result<Usage, UsageError> {
    let prompt_tokens = optional_integer(mapping, "prompt_tokens")?.unwrap_or(0);
    let completion_tokens = optional_integer(mapping, "completion_tokens")?.unwrap_or(0);
    let total_tokens = optional_integer(mapping, "total_tokens")?.unwrap_or(0);
    let mut prompt_tokens_details = object_field(mapping, "prompt_tokens_details")?.cloned();
    apply_cache_aliases(&mut prompt_tokens_details, mapping)?;
    let mut completion_tokens_details =
        object_field(mapping, "completion_tokens_details")?.cloned();
    apply_reasoning(&mut completion_tokens_details, mapping, completion_tokens)?;
    let server_tool_use = object_field(mapping, "server_tool_use")?.cloned();
    let mut extras = mapping.clone();
    extras.shift_remove("prompt_tokens");
    extras.shift_remove("completion_tokens");
    extras.shift_remove("total_tokens");
    extras.shift_remove("reasoning_tokens");
    extras.shift_remove("prompt_tokens_details");
    extras.shift_remove("completion_tokens_details");
    extras.shift_remove("server_tool_use");
    extras.shift_remove("cost");
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details,
        completion_tokens_details,
        server_tool_use,
        cost: usage_cost(mapping)?,
        extras,
    })
}

fn apply_reasoning(
    details: &mut Option<IndexMap<String, Json>>,
    mapping: &IndexMap<String, Json>,
    completion_tokens: i64,
) -> Result<(), UsageError> {
    let Some(reasoning) = optional_integer(mapping, "reasoning_tokens")? else {
        return Ok(());
    };
    let parsed = details.get_or_insert_default();
    if detail_number(parsed, "reasoning_tokens").is_none() {
        parsed.insert(
            "reasoning_tokens".to_owned(),
            Json::Number(Number::from(reasoning)),
        );
    }
    let completion_reported = !matches!(mapping.get("completion_tokens"), None | Some(Json::Null));
    if detail_number(parsed, "text_tokens").is_some() || !completion_reported {
        return Ok(());
    }
    let mut text_tokens =
        completion_tokens
            .checked_sub(reasoning)
            .ok_or(UsageError::IntegerOverflow {
                field: "completion_tokens",
            })?;
    for name in ["image_tokens", "audio_tokens"] {
        if let Some(tokens) = detail_number(parsed, name) {
            text_tokens = text_tokens
                .checked_sub(tokens)
                .ok_or(UsageError::IntegerOverflow {
                    field: "completion_tokens",
                })?;
        }
    }
    parsed.insert(
        "text_tokens".to_owned(),
        Json::Number(Number::from(text_tokens.max(0))),
    );
    Ok(())
}

fn apply_cache_aliases(
    details: &mut Option<IndexMap<String, Json>>,
    mapping: &IndexMap<String, Json>,
) -> Result<(), UsageError> {
    for (alias, target) in [
        ("prompt_cache_hit_tokens", "cached_tokens"),
        ("cache_read_input_tokens", "cached_tokens"),
        ("cache_creation_input_tokens", "cache_write_tokens"),
    ] {
        if let Some(value) = optional_integer(mapping, alias)? {
            details
                .get_or_insert_default()
                .insert(target.to_owned(), Json::Number(Number::from(value)));
        }
    }
    Ok(())
}

fn usage_cost(mapping: &IndexMap<String, Json>) -> Result<Option<Decimal>, UsageError> {
    match mapping.get("cost") {
        Some(Json::Number(number)) => decimal_number(number, "cost").map(Some),
        Some(Json::Null) | None => Ok(None),
        Some(_) => Err(UsageError::NotANumber { field: "cost" }),
    }
}

fn object_field<'mapping>(
    fields: &'mapping IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'mapping IndexMap<String, Json>>, UsageError> {
    let Some(value) = fields.get(name) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Object(nested) => Ok(Some(nested)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(UsageError::NotAnObject { field: name })
        }
    }
}

fn optional_integer(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<i64>, UsageError> {
    match fields.get(name) {
        Some(Json::Number(number)) => integer_number(number, name).map(Some),
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(UsageError::NotAnInteger { field: name })
        }
    }
}

fn integer_number(number: &Number, name: &'static str) -> Result<i64, UsageError> {
    let value = decimal_number(number, name)?;
    if !value.fract().is_zero() {
        return Err(UsageError::NotAnInteger { field: name });
    }
    i64::try_from(value).map_err(|_error| UsageError::IntegerOverflow { field: name })
}

fn detail_number(fields: &IndexMap<String, Json>, name: &str) -> Option<i64> {
    match fields.get(name) {
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// A JSON float may arrive in exponent notation, which Decimal::from_str refuses; from_scientific reads it.
fn decimal_number(number: &Number, name: &'static str) -> Result<Decimal, UsageError> {
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        Decimal::from_str(&text)
    };
    parsed.ok().ok_or(UsageError::NotANumber { field: name })
}
