use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use betterlite_models::cost_usage::transcription::{
    TranscriptionUsage, transform_transcription_usage,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::fmt::Display;
use thiserror::Error;

pub mod parse;
pub mod project;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: Option<IndexMap<String, Json>>,
    pub completion_tokens_details: Option<IndexMap<String, Json>>,
    pub server_tool_use: Option<IndexMap<String, Json>>,
    pub cost: Option<Decimal>,
    pub extras: IndexMap<String, Json>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum UsageError {
    #[error("the usage field {field} must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the usage field {field} does not fit a signed 64-bit integer")]
    IntegerOverflow { field: &'static str },
    #[error("the usage field {field} must be an object")]
    NotAnObject { field: &'static str },
    #[error("the usage field {field} must be a number")]
    NotANumber { field: &'static str },
    #[error("the provider usage object could not be converted: {reason}")]
    ProviderConversion { reason: String },
}

#[derive(Clone, Debug)]
pub enum ResponseUsage<'usage> {
    Absent,
    Mapping(IndexMap<String, Json>),
    Usage(&'usage Usage),
}

#[derive(Clone, Copy, Debug)]
pub enum NativeResponseUsage<'native> {
    Responses(&'native Usage),
    Transcription(&'native TranscriptionUsage),
}

// The Responses API usage conversion B906 owns; betterlite-core depends on this crate, so callers inject it.
pub type ResponsesConverter<'converter> =
    &'converter dyn Fn(&IndexMap<String, Json>) -> Result<Usage, UsageError>;

pub fn get_usage_from_response_obj(
    response: ResponseUsage<'_>,
    combined: Option<&Usage>,
    convert_responses: ResponsesConverter<'_>,
) -> Result<Usage, UsageError> {
    if let Some(base) = combined {
        return Ok(base.clone());
    }
    match response {
        ResponseUsage::Absent => Ok(Usage::default()),
        ResponseUsage::Usage(usage) => Ok(usage.clone()),
        ResponseUsage::Mapping(mapping) => converted_usage(&mapping, convert_responses)?
            .map_or_else(|| parse::usage_from_mapping(&mapping), Ok),
    }
}

pub fn get_usage_as_dict(
    usage_response: ResponseUsage<'_>,
    combined_usage: Option<&Usage>,
    converter: ResponsesConverter<'_>,
) -> Result<Json, UsageError> {
    if let Some(base) = combined_usage {
        return Ok(project::usage_json(base));
    }
    if let ResponseUsage::Mapping(mapping) = &usage_response
        && !provider_usage(mapping)?
    {
        return Ok(Json::Object(mapping.clone()));
    }
    if matches!(usage_response, ResponseUsage::Absent) {
        return Ok(zero_usage_json());
    }
    get_usage_from_response_obj(usage_response, None, converter)
        .map(|usage| project::usage_json(&usage))
}

fn zero_usage_json() -> Json {
    let record: IndexMap<String, Json> = ["prompt_tokens", "completion_tokens", "total_tokens"]
        .into_iter()
        .map(|name| (name.to_owned(), Json::Number(Number::from(0_i64))))
        .collect();
    Json::Object(record)
}

fn provider_usage(mapping: &IndexMap<String, Json>) -> Result<bool, UsageError> {
    if mapping.contains_key("input_tokens") && mapping.contains_key("output_tokens") {
        return Ok(true);
    }
    let parsed = parse_interactions(mapping)?;
    Ok(is_interactions_usage_object(&parsed))
}

fn converted_usage(
    mapping: &IndexMap<String, Json>,
    convert_responses: ResponsesConverter<'_>,
) -> Result<Option<Usage>, UsageError> {
    if !provider_usage(mapping)? {
        return Ok(None);
    }
    if mapping.contains_key("input_tokens") && mapping.contains_key("output_tokens") {
        return convert_responses(mapping).map(Some);
    }
    let cost =
        transform_interactions_usage(&parse_interactions(mapping)?).map_err(conversion_error)?;
    let mut usage = Usage::from(&cost);
    let cached = cost
        .cache_read_tokens
        .map_or(Json::Null, |tokens| Json::Number(Number::from(tokens)));
    usage
        .extras
        .insert("cache_read_input_tokens".to_owned(), cached);
    Ok(Some(usage))
}

fn parse_interactions(mapping: &IndexMap<String, Json>) -> Result<InteractionsUsage, UsageError> {
    serde_json::from_str(&serde_json::to_string(mapping).map_err(conversion_error)?)
        .map_err(conversion_error)
}

fn conversion_error<E>(error: E) -> UsageError
where
    E: Display,
{
    UsageError::ProviderConversion {
        reason: error.to_string(),
    }
}

#[must_use]
pub fn transform_usage_objects(usage: NativeResponseUsage<'_>) -> Option<Usage> {
    match usage {
        NativeResponseUsage::Responses(converted) => Some(converted.clone()),
        NativeResponseUsage::Transcription(native) => {
            transform_transcription_usage(native).map(|cost| Usage::from(&cost))
        }
    }
}

impl From<&CostUsage> for Usage {
    fn from(cost: &CostUsage) -> Self {
        Self {
            prompt_tokens: cost.prompt_tokens,
            completion_tokens: cost.completion_tokens,
            total_tokens: cost.total_tokens,
            prompt_tokens_details: prompt_details(cost),
            completion_tokens_details: completion_details(cost),
            server_tool_use: None,
            cost: None,
            extras: IndexMap::new(),
        }
    }
}

fn prompt_details(cost: &CostUsage) -> Option<IndexMap<String, Json>> {
    if cost.prompt_details.is_none() && cost.cache_read_tokens.is_none() {
        return None;
    }
    let mut record = IndexMap::new();
    if let Some(details) = &cost.prompt_details {
        insert_number(&mut record, "text_tokens", details.text_tokens);
        insert_number(&mut record, "audio_tokens", details.audio_tokens);
        insert_number(&mut record, "image_tokens", details.image_tokens);
        insert_number(&mut record, "video_tokens", details.video_tokens);
        insert_number(
            &mut record,
            "web_search_requests",
            details.web_search_requests,
        );
    }
    insert_number(&mut record, "cached_tokens", cost.cache_read_tokens);
    Some(record)
}

fn completion_details(cost: &CostUsage) -> Option<IndexMap<String, Json>> {
    let details = cost.completion_details.as_ref()?;
    let mut record = IndexMap::new();
    insert_number(&mut record, "text_tokens", details.text_tokens);
    insert_number(&mut record, "audio_tokens", details.audio_tokens);
    insert_number(&mut record, "image_tokens", details.image_tokens);
    insert_number(&mut record, "video_tokens", details.video_tokens);
    insert_number(&mut record, "reasoning_tokens", details.reasoning_tokens);
    Some(record)
}

fn insert_number(record: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    if let Some(number) = value {
        record.insert(name.to_owned(), Json::Number(Number::from(number)));
    }
}

mod tests;
