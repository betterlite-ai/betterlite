use std::collections::BTreeMap;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage as PricingUsage, UsageNumber};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Map, Number, Value};

use crate::api::responses_usage::{ResponsesUsageError, UsageInput, normalize_usage};

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

// The logging usage the response carries, read as the typed block cost_per_token prices: details and extensions become typed counters.
pub fn pricing_usage(usage: &Usage) -> Result<PricingUsage, UsageConversionError> {
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
    let converted = normalize_usage(UsageInput::Mapping(&Value::Object(fields)))?;
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
        } else if let Some(flag) = extension.as_bool() {
            numeric_extras.insert(name.clone(), UsageNumber::Boolean(flag));
        } else if let Some(float) = extension.as_f64() {
            let number =
                Decimal::try_from(float).map_err(|source| UsageConversionError::Decimal {
                    name: name.clone(),
                    source,
                })?;
            numeric_extras.insert(name.clone(), UsageNumber::Decimal(number));
        }
    }
    Ok(PricingUsage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details: converted.prompt_tokens_details,
        completion_tokens_details: converted.completion_tokens_details,
        numeric_extras,
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

// A websocket partition carries the typed usage block; the response view the prepare stage reads wants the logging shape.
pub fn logging_usage(usage: &PricingUsage) -> Result<Usage, UsageConversionError> {
    let prompt = usage.prompt_tokens_details.as_ref();
    let completion = usage.completion_tokens_details.as_ref();
    let mut prompt_details = prompt.and_then(|details| {
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
    });
    for (name, seconds) in prompt.iter().flat_map(|details| {
        [
            ("video_length_seconds", details.video_length_seconds),
            ("audio_length_seconds", details.audio_length_seconds),
        ]
    }) {
        if let Some(found) = seconds {
            prompt_details
                .get_or_insert_with(IndexMap::new)
                .insert(name.to_owned(), Json::Number(found.to_string().parse()?));
        }
    }
    let completion_details = completion.and_then(|details| {
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
    });
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
    Ok(Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: prompt_details,
        completion_tokens_details: completion_details,
        server_tool_use: None,
        cost,
        extras,
    })
}

mod tests;
