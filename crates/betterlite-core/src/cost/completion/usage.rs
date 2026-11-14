use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage as PricingUsage;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::responses_usage::ResponsesUsageError;

use extras::decoded;

pub mod pricing;

mod extras;

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

fn counters(entries: &[(&str, Option<i64>)]) -> Option<IndexMap<String, Json>> {
    let record: IndexMap<String, Json> = entries
        .iter()
        .filter_map(|(name, count)| {
            count.map(|found| ((*name).to_owned(), Json::Number(Number::from(found))))
        })
        .collect();
    (!record.is_empty()).then_some(record)
}

// A duration alone still opens the prompt details, since the prepare stage reads the block as present.
fn prompt_details(
    usage: &PricingUsage,
) -> Result<Option<IndexMap<String, Json>>, UsageConversionError> {
    let Some(details) = usage.prompt_tokens_details.as_ref() else {
        return Ok(None);
    };
    let mut record = counters(&[
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
    ]);
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

// A websocket partition carries the typed usage block; the response view the prepare stage reads wants the logging shape.
pub fn logging_usage(usage: &PricingUsage) -> Result<Usage, UsageConversionError> {
    let prompt_record = prompt_details(usage)?;
    let completion_record = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| {
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
    let (cost, extras) = decoded(usage)?;
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
