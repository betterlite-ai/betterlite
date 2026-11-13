use betterlite_anthropic::usage::calculate_usage;
use betterlite_anthropic::usage::is_anthropic_usage_object;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::TokenCount;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use serde_json::Number;

use super::BillingUsageError as Error;
use super::{BillingUsage, NoReasoningCounter};

use crate::api::responses_usage::{
    Usage as ResponsesUsage, UsageInput, normalize_usage, transform_response_api_usage,
};
mod anthropic;

pub(super) fn mapping_usage(mapping: &IndexMap<String, Json>) -> Result<Option<Usage>, Error> {
    let object = Json::Object(mapping.clone());
    if is_anthropic_usage_object(&object) {
        let counter = NoReasoningCounter;
        let anthropic =
            calculate_usage(&object, None, None, None, &counter).map_err(Error::Anthropic)?;
        return Ok(Some(anthropic::from_usage(&anthropic)));
    }
    // B906 converts a Responses usage only when both keys are present on the same object.
    if mapping.contains_key("input_tokens") && mapping.contains_key("output_tokens") {
        let text = serde_json::to_string(&object).map_err(Error::Json)?;
        let value = serde_json::from_str(&text).map_err(Error::Json)?;
        let converted = transform_response_api_usage(Some(&value)).map_err(Error::Responses)?;
        return from_responses_usage(&converted).map(Some);
    }
    if let Some(interactions) = interactions_candidate(mapping)? {
        return interactions_from(&interactions).map(Some);
    }
    usage_from_mapping(mapping).map(Some).map_err(Error::Usage)
}
pub(super) fn interactions_usage(mapping: &IndexMap<String, Json>) -> Result<Usage, Error> {
    let parsed = interactions_candidate(mapping)?.ok_or(Error::Projection {
        kind: "interactions",
    })?;
    interactions_from(&parsed)
}
fn interactions_candidate(
    mapping: &IndexMap<String, Json>,
) -> Result<Option<InteractionsUsage>, Error> {
    if !mapping.contains_key("total_input_tokens") && !mapping.contains_key("total_output_tokens") {
        return Ok(None);
    }
    let text = serde_json::to_string(&Json::Object(mapping.clone())).map_err(Error::Json)?;
    let parsed: InteractionsUsage = serde_json::from_str(&text).map_err(Error::Json)?;
    Ok(is_interactions_usage_object(&parsed).then_some(parsed))
}
fn interactions_from(usage: &InteractionsUsage) -> Result<Usage, Error> {
    let cost = transform_interactions_usage(usage).map_err(Error::Interactions)?;
    let mut converted = Usage::from(&cost);
    let cached = cost
        .cache_read_tokens
        .map_or(Json::Null, |tokens| Json::Number(Number::from(tokens)));
    converted
        .extras
        .insert("cache_read_input_tokens".to_owned(), cached);
    Ok(converted)
}
pub(super) fn responses_usage(usage_object: &ResponseApiUsage) -> Result<Usage, Error> {
    let converted_usage =
        normalize_usage(UsageInput::Response(usage_object)).map_err(Error::Responses)?;
    from_responses_usage(&converted_usage)
}
fn from_responses_usage(converted: &ResponsesUsage) -> Result<Usage, Error> {
    let mut extras = IndexMap::new();
    for (name, value) in &converted.extensions {
        let field: Json = serde_json::from_value(value.clone()).map_err(Error::Json)?;
        extras.insert(name.clone(), field);
    }
    Ok(Usage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details: converted
            .prompt_tokens_details
            .as_ref()
            .map(prompt_details_from_responses),
        completion_tokens_details: converted
            .completion_tokens_details
            .as_ref()
            .map(completion_details),
        server_tool_use: None,
        cost: converted.cost,
        extras,
    })
}
fn prompt_details_from_responses(details: &PromptTokensDetails) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    for (name, value) in [
        ("text_tokens", details.text_tokens),
        ("audio_tokens", details.audio_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", details.video_tokens),
        ("cached_tokens", details.cached_tokens),
        ("cache_write_tokens", details.cache_write_tokens),
        ("cache_creation_tokens", details.cache_creation_tokens),
        ("web_search_requests", details.web_search_requests),
        (
            "google_maps_grounding_requests",
            details.google_maps_grounding_requests,
        ),
        ("tool_use_tokens", details.tool_use_tokens),
        ("query_count", details.query_count),
    ] {
        insert_count(&mut record, name, value);
    }
    record
}
fn completion_details(details: &CompletionTokensDetails) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    for (name, value) in [
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
    ] {
        insert_count(&mut record, name, value);
    }
    record
}
pub(super) fn decisions_usage(usage: &BillingUsage<'_>) -> Result<Option<Usage>, Error> {
    match usage {
        BillingUsage::Absent | BillingUsage::Null => Ok(None),
        BillingUsage::DecisionsIr(found) => decision_usage(
            &found.input_tokens,
            &found.output_tokens,
            &found.cached_tokens,
            &found.cache_write_tokens,
        )
        .map(Some),
        BillingUsage::DecisionsOpenAi(found) => decision_usage(
            &found.input_tokens,
            &found.output_tokens,
            &found.input_tokens_details.cached_tokens,
            &found.input_tokens_details.cache_write_tokens,
        )
        .map(Some),
        BillingUsage::Chat(_)
        | BillingUsage::Response(_)
        | BillingUsage::Transcription(_)
        | BillingUsage::Interactions(_)
        | BillingUsage::Mapping(_)
        | BillingUsage::TypedRecord(_) => Err(Error::Projection { kind: "decision" }),
    }
}

// LiteLLM recomputes the total from input and output and never adds Decisions output reasoning.
fn decision_usage(
    input: &TokenCount,
    output: &TokenCount,
    cached: &TokenCount,
    write: &TokenCount,
) -> Result<Usage, Error> {
    let prompt_tokens = input.0.as_i64().ok_or(Error::Token {
        field: "input_tokens",
    })?;
    let completion_tokens = output.0.as_i64().ok_or(Error::Token {
        field: "output_tokens",
    })?;
    let cached_tokens = cached.0.as_i64().ok_or(Error::Token {
        field: "cached_tokens",
    })?;
    let write_tokens = write.0.as_i64().ok_or(Error::Token {
        field: "cache_write_tokens",
    })?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(Error::Token {
            field: "total_tokens",
        })?;
    let mut details = IndexMap::new();
    insert_count(&mut details, "cached_tokens", Some(cached_tokens));
    insert_count(&mut details, "cache_write_tokens", Some(write_tokens));
    insert_count(&mut details, "cache_creation_tokens", Some(write_tokens));
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: Some(details),
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    })
}
fn insert_count(record: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    if let Some(count) = value {
        record.insert(name.to_owned(), Json::Number(Number::from(count)));
    }
}
