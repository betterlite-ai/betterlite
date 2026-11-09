use betterlite_anthropic::usage::calculate_usage;
use betterlite_anthropic::usage::is_anthropic_usage_object;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use serde_json::Number;

use super::{BillingUsageError, NoReasoningCounter};

use crate::api::responses_usage::{
    Usage as ResponsesUsage, UsageInput, normalize_usage, transform_response_api_usage,
};

mod anthropic;
pub(super) mod decisions;

pub(super) fn mapping_usage(
    mapping: &IndexMap<String, Json>,
) -> Result<Option<Usage>, BillingUsageError> {
    let object = Json::Object(mapping.clone());
    if is_anthropic_usage_object(&object) {
        let counter = NoReasoningCounter;
        let anthropic = calculate_usage(&object, None, None, None, &counter)
            .map_err(|source| BillingUsageError::Anthropic { source })?;
        return Ok(Some(anthropic::from_usage(&anthropic)));
    }
    // B906 converts a Responses usage only when both keys are present on the same object.
    if mapping.contains_key("input_tokens") && mapping.contains_key("output_tokens") {
        let text =
            serde_json::to_string(&object).map_err(|source| BillingUsageError::Json { source })?;
        let value =
            serde_json::from_str(&text).map_err(|source| BillingUsageError::Json { source })?;
        let converted = transform_response_api_usage(Some(&value))
            .map_err(|source| BillingUsageError::Responses { source })?;
        return from_responses_usage(&converted).map(Some);
    }
    if let Some(interactions) = interactions_candidate(mapping)? {
        return interactions_from(&interactions).map(Some);
    }
    usage_from_mapping(mapping)
        .map(Some)
        .map_err(|source| BillingUsageError::Usage { source })
}

pub(super) fn interactions_usage(
    mapping: &IndexMap<String, Json>,
) -> Result<Usage, BillingUsageError> {
    let parsed = interactions_candidate(mapping)?.ok_or(BillingUsageError::Projection {
        kind: "interactions",
    })?;
    interactions_from(&parsed)
}

fn interactions_candidate(
    mapping: &IndexMap<String, Json>,
) -> Result<Option<InteractionsUsage>, BillingUsageError> {
    if !mapping.contains_key("total_input_tokens") && !mapping.contains_key("total_output_tokens") {
        return Ok(None);
    }
    let text = serde_json::to_string(&Json::Object(mapping.clone()))
        .map_err(|source| BillingUsageError::Json { source })?;
    let parsed: InteractionsUsage =
        serde_json::from_str(&text).map_err(|source| BillingUsageError::Json { source })?;
    Ok(is_interactions_usage_object(&parsed).then_some(parsed))
}

fn interactions_from(usage: &InteractionsUsage) -> Result<Usage, BillingUsageError> {
    let cost = transform_interactions_usage(usage)
        .map_err(|source| BillingUsageError::Interactions { source })?;
    let mut converted = Usage::from(&cost);
    let cached = cost
        .cache_read_tokens
        .map_or(Json::Null, |tokens| Json::Number(Number::from(tokens)));
    converted
        .extras
        .insert("cache_read_input_tokens".to_owned(), cached);
    Ok(converted)
}

pub(super) fn responses_usage(usage: &ResponseApiUsage) -> Result<Usage, BillingUsageError> {
    let converted = normalize_usage(UsageInput::Response(usage))
        .map_err(|source| BillingUsageError::Responses { source })?;
    from_responses_usage(&converted)
}

fn from_responses_usage(converted: &ResponsesUsage) -> Result<Usage, BillingUsageError> {
    let mut extras = IndexMap::new();
    for (name, value) in &converted.extensions {
        let field: Json = serde_json::from_value(value.clone())
            .map_err(|source| BillingUsageError::Json { source })?;
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
    insert_count(&mut record, "text_tokens", details.text_tokens);
    insert_count(&mut record, "audio_tokens", details.audio_tokens);
    insert_count(&mut record, "image_tokens", details.image_tokens);
    insert_count(&mut record, "video_tokens", details.video_tokens);
    insert_count(&mut record, "cached_tokens", details.cached_tokens);
    insert_count(
        &mut record,
        "cache_write_tokens",
        details.cache_write_tokens,
    );
    insert_count(
        &mut record,
        "cache_creation_tokens",
        details.cache_creation_tokens,
    );
    insert_count(
        &mut record,
        "web_search_requests",
        details.web_search_requests,
    );
    insert_count(
        &mut record,
        "google_maps_grounding_requests",
        details.google_maps_grounding_requests,
    );
    insert_count(&mut record, "tool_use_tokens", details.tool_use_tokens);
    insert_count(&mut record, "query_count", details.query_count);
    record
}

fn completion_details(details: &CompletionTokensDetails) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    insert_count(&mut record, "text_tokens", details.text_tokens);
    insert_count(&mut record, "audio_tokens", details.audio_tokens);
    insert_count(&mut record, "image_tokens", details.image_tokens);
    insert_count(&mut record, "video_tokens", details.video_tokens);
    insert_count(&mut record, "reasoning_tokens", details.reasoning_tokens);
    insert_count(
        &mut record,
        "accepted_prediction_tokens",
        details.accepted_prediction_tokens,
    );
    insert_count(
        &mut record,
        "rejected_prediction_tokens",
        details.rejected_prediction_tokens,
    );
    record
}

fn insert_count(record: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    if let Some(count) = value {
        record.insert(name.to_owned(), Json::Number(Number::from(count)));
    }
}
