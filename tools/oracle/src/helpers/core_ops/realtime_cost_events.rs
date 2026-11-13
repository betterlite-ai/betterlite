use std::collections::BTreeMap;

use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_core::cost::realtime::{
    Event, RealtimeBilling, realtime_stream, responses_ws, transcription,
};
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber, usage_sum,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};

type Failure = Box<crate::helpers::HelperError>;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Collect(EventsInput),
    TranscriptionModel(EventsInput),
}

#[derive(Deserialize)]
pub(crate) struct EventsInput {
    events: Vec<Event>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Collect(fields) => collect(&fields.events),
            Input::TranscriptionModel(fields) => Ok(json!({
                "model": transcription::session::model_name(&fields.events).map_err(|error| failure(&error))?,
            })),
        }
    }
}

fn failure(error: &RealtimeCostError) -> Failure {
    Box::new(crate::helpers::HelperError {
        class: "RealtimeCostError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn collect(events: &[Event]) -> crate::helpers::HelperResult {
    let realtime_usages =
        realtime_stream::collect_usage(events).map_err(|error| failure(&error))?;
    let realtime_combined = usage_sum(&realtime_usages)
        .map_err(|source| failure(&RealtimeCostError::Aggregation { source }))?;
    let responses_usages = responses_ws::collect_usage(events).map_err(|error| failure(&error))?;
    let responses_combined = usage_sum(&responses_usages)
        .map_err(|source| failure(&RealtimeCostError::Aggregation { source }))?;
    let partition =
        responses_ws::partition_by_service_tier(events).map_err(|error| failure(&error))?;
    let split = responses_ws::split_by_service_tier(&RealtimeBilling {
        results: events.to_vec(),
        usage: responses_combined.clone(),
        service_tier: None,
    })
    .map_err(|error| failure(&error))?;
    Ok(json!({
        "realtime": {
            "usages": realtime_usages
                .iter()
                .map(usage_value)
                .collect::<Result<Vec<_>, Failure>>()?,
            "combined": usage_value(&realtime_combined)?,
        },
        "responses_ws": {
            "usages": responses_usages
                .iter()
                .map(usage_value)
                .collect::<Result<Vec<_>, Failure>>()?,
            "combined": usage_value(&responses_combined)?,
            "partitions": partition_values(&partition),
            "split": split_value(split.as_deref())?,
        },
    }))
}

fn usage_value(usage: &Usage) -> Result<Value, Failure> {
    let prompt_details = usage
        .prompt_tokens_details
        .as_ref()
        .map_or_else(|| Ok(Value::Null), prompt_details_value)?;
    let completion_details = usage
        .completion_tokens_details
        .as_ref()
        .map_or(Value::Null, completion_details_value);
    let mut fields = BTreeMap::new();
    fields.insert("prompt_tokens".to_owned(), json!(usage.prompt_tokens));
    fields.insert(
        "completion_tokens".to_owned(),
        json!(usage.completion_tokens),
    );
    fields.insert("total_tokens".to_owned(), json!(usage.total_tokens));
    fields.insert("prompt_tokens_details".to_owned(), prompt_details);
    fields.insert("completion_tokens_details".to_owned(), completion_details);
    fields.insert("cost".to_owned(), cost_value(&usage.numeric_extras)?);
    fields.insert(
        "numeric_extras".to_owned(),
        numeric_extras_value(&usage.numeric_extras)?,
    );
    Ok(json!(fields))
}

fn prompt_details_value(details: &PromptTokensDetails) -> Result<Value, Failure> {
    let cached_details = details.cached_tokens_details.as_ref().map(|cached| {
        json!({
            "text_tokens": cached.text_tokens,
            "audio_tokens": cached.audio_tokens,
            "image_tokens": cached.image_tokens,
        })
    });
    Ok(json!({
        "text_tokens": details.text_tokens,
        "audio_tokens": details.audio_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
        "cache_write_tokens": details.cache_write_tokens,
        "cache_creation_tokens": details.cache_creation_tokens,
        "cached_tokens": details.cached_tokens,
        "cached_tokens_details": cached_details,
        "web_search_requests": details.web_search_requests,
        "google_maps_grounding_requests": details.google_maps_grounding_requests,
        "tool_use_tokens": details.tool_use_tokens,
        "character_count": details.character_count,
        "image_count": details.image_count,
        "video_length_seconds": decimal_value(details.video_length_seconds)?,
        "audio_length_seconds": decimal_value(details.audio_length_seconds)?,
        "query_count": details.query_count,
    }))
}

fn completion_details_value(details: &CompletionTokensDetails) -> Value {
    json!({
        "text_tokens": details.text_tokens,
        "reasoning_tokens": details.reasoning_tokens,
        "accepted_prediction_tokens": details.accepted_prediction_tokens,
        "rejected_prediction_tokens": details.rejected_prediction_tokens,
        "audio_tokens": details.audio_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
    })
}

fn cost_value(extras: &BTreeMap<String, UsageNumber>) -> Result<Value, Failure> {
    extras
        .get("cost")
        .map_or_else(|| Ok(Value::Null), number_value)
}

fn numeric_extras_value(extras: &BTreeMap<String, UsageNumber>) -> Result<Value, Failure> {
    let mut rendered = BTreeMap::new();
    for (name, value) in extras.iter().filter(|(name, _)| *name != "cost") {
        rendered.insert(name.clone(), number_value(value)?);
    }
    Ok(json!(rendered))
}

fn number_value(value: &UsageNumber) -> Result<Value, Failure> {
    match value {
        UsageNumber::Boolean(inner) => Ok(json!(inner)),
        UsageNumber::Integer(inner) => Ok(json!(inner)),
        UsageNumber::Decimal(inner) => decimal_to_value(*inner, "decimal extra"),
    }
}

fn decimal_value(value: Option<Decimal>) -> Result<Value, Failure> {
    value.map_or(Ok(Value::Null), |decimal| {
        decimal_to_value(decimal, "decimal detail")
    })
}

fn decimal_to_value(decimal: Decimal, label: &str) -> Result<Value, Failure> {
    decimal.to_f64().map(|float| json!(float)).ok_or_else(|| {
        Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("{label} {decimal} has no f64 representation"),
        })
    })
}

fn partition_values(partition: &IndexMap<Option<String>, Vec<Event>>) -> Vec<Value> {
    partition
        .iter()
        .map(|(service_tier, results)| {
            json!({
                "service_tier": service_tier,
                "results": results,
            })
        })
        .collect()
}

fn split_value(parts: Option<&[RealtimeBilling]>) -> Result<Value, Failure> {
    let Some(found) = parts else {
        return Ok(Value::Null);
    };
    let mut rendered = Vec::with_capacity(found.len());
    for part in found {
        rendered.push(json!({
            "service_tier": part.service_tier,
            "usage": usage_value(&part.usage)?,
            "results": part.results,
        }));
    }
    Ok(Value::Array(rendered))
}
