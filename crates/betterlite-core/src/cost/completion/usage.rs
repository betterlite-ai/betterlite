use std::collections::BTreeMap;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage as PricingUsage, UsageNumber,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Map, Number, Value};

use crate::api::responses_usage::{
    ResponsesUsageError, Usage as NormalizedUsage, UsageInput, normalize_usage,
};

#[derive(Debug, thiserror::Error)]
pub enum UsageConversionError {
    #[error("usage JSON conversion failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Normalize(#[from] ResponsesUsageError),
    #[error("usage extension {name} is not a decimal number: {source}")]
    Decimal {
        name: String,
        #[source]
        source: rust_decimal::Error,
    },
}

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

fn counters(entries: &[(&str, Option<i64>)]) -> Option<IndexMap<String, Json>> {
    let record: IndexMap<String, Json> = entries
        .iter()
        .filter_map(|(name, count)| {
            count.map(|found| ((*name).to_owned(), Json::Number(Number::from(found))))
        })
        .collect();
    (!record.is_empty()).then_some(record)
}

fn prompt_counters(details: &PromptTokensDetails) -> Option<IndexMap<String, Json>> {
    counters(&[
        ("text_tokens", details.text_tokens),
        ("audio_tokens", details.audio_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", details.video_tokens),
        ("web_search_requests", details.web_search_requests),
        (
            "google_maps_grounding_requests",
            details.google_maps_grounding_requests,
        ),
        ("tool_use_tokens", details.tool_use_tokens),
        ("character_count", details.character_count),
        ("image_count", details.image_count),
        ("query_count", details.query_count),
        ("cache_write_tokens", details.cache_write_tokens),
        ("cache_creation_tokens", details.cache_creation_tokens),
        ("cached_tokens", details.cached_tokens),
    ])
}

// A duration alone still opens the prompt details, since the prepare stage reads the block as present.
fn prompt_details(
    usage: &PricingUsage,
) -> Result<Option<IndexMap<String, Json>>, UsageConversionError> {
    let Some(details) = usage.prompt_tokens_details.as_ref() else {
        return Ok(None);
    };
    let mut record = prompt_counters(details);
    for (name, seconds) in [
        ("video_length_seconds", details.video_length_seconds),
        ("audio_length_seconds", details.audio_length_seconds),
    ] {
        if let Some(found) = seconds {
            record
                .get_or_insert_with(IndexMap::new)
                .insert(name.to_owned(), Json::Number(found.to_string().parse()?));
        }
    }
    Ok(record)
}

fn completion_counters(details: &CompletionTokensDetails) -> Option<IndexMap<String, Json>> {
    counters(&[
        ("text_tokens", details.text_tokens),
        ("audio_tokens", details.audio_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", details.video_tokens),
        ("reasoning_tokens", details.reasoning_tokens),
        (
            "accepted_prediction_tokens",
            details.accepted_prediction_tokens,
        ),
        (
            "rejected_prediction_tokens",
            details.rejected_prediction_tokens,
        ),
    ])
}

// The cost extension becomes the stated cost; every other extension keeps its JSON kind.
fn decoded_extras(
    usage: &PricingUsage,
) -> Result<(Option<Decimal>, IndexMap<String, Json>), UsageConversionError> {
    let mut cost = None;
    let mut extras = IndexMap::new();
    for (name, number) in &usage.numeric_extras {
        match number {
            UsageNumber::Decimal(found) if name == "cost" => cost = Some(*found),
            UsageNumber::Decimal(found) => {
                extras.insert(name.clone(), Json::Number(found.to_string().parse()?));
            }
            UsageNumber::Integer(found) => {
                extras.insert(name.clone(), Json::Number(Number::from(*found)));
            }
            UsageNumber::Boolean(flag) => {
                extras.insert(name.clone(), Json::Bool(*flag));
            }
        }
    }
    Ok((cost, extras))
}

// A websocket partition carries the typed usage block; the response view the prepare stage reads wants the logging shape.
pub fn logging_usage(usage: &PricingUsage) -> Result<Usage, UsageConversionError> {
    let prompt_record = prompt_details(usage)?;
    let completion_record = usage
        .completion_tokens_details
        .as_ref()
        .and_then(completion_counters);
    let (cost, extras) = decoded_extras(usage)?;
    Ok(Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: prompt_record,
        completion_tokens_details: completion_record,
        server_tool_use: None,
        cost,
        extras,
    })
}

mod tests;
