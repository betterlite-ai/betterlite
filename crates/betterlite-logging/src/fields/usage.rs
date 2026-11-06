use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::transcription::{
    TranscriptionUsage, transform_transcription_usage,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
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

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum UsageError {
    #[error("the usage field {field} must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the usage field {field} does not fit a signed 64-bit integer")]
    IntegerOverflow { field: &'static str },
    #[error("the usage field {field} must be an object")]
    NotAnObject { field: &'static str },
    #[error("the usage field {field} must be a number")]
    NotANumber { field: &'static str },
}

// The usage value a response object carries; an endpoint owner hands over its native usage as the typed Usage.
#[derive(Clone, Debug)]
pub enum ResponseUsage<'usage> {
    Absent,
    Mapping(IndexMap<String, Json>),
    Usage(&'usage Usage),
}

// The usage a native response owns; the projection decides only the chat shape logging stores.
#[derive(Clone, Copy, Debug)]
pub enum NativeResponseUsage<'native> {
    Responses(&'native Usage),
    Transcription(&'native TranscriptionUsage),
}

// StandardLoggingPayloadSetup.get_usage_from_response_obj: a combined usage wins, and an absent usage reads as zero tokens.
pub fn get_usage_from_response_obj(
    response: ResponseUsage<'_>,
    combined: Option<&Usage>,
) -> Result<Usage, UsageError> {
    if let Some(base) = combined {
        return Ok(base.clone());
    }
    match response {
        ResponseUsage::Absent => Ok(Usage::default()),
        ResponseUsage::Usage(usage) => Ok(usage.clone()),
        ResponseUsage::Mapping(mapping) => parse::usage_from_mapping(&mapping),
    }
}

// StandardLoggingPayloadSetup.get_usage_as_dict: an ordinary mapping is logged as sent, every other source as the model dump.
#[must_use]
pub fn get_usage_as_dict(response: ResponseUsage<'_>, combined: Option<&Usage>) -> Json {
    if let Some(base) = combined {
        return project::usage_json(base);
    }
    projected_usage_json(response)
}

fn projected_usage_json(response: ResponseUsage<'_>) -> Json {
    match response {
        ResponseUsage::Absent => Json::Object(IndexMap::from([
            ("prompt_tokens".to_owned(), Json::Number(Number::from(0))),
            (
                "completion_tokens".to_owned(),
                Json::Number(Number::from(0)),
            ),
            ("total_tokens".to_owned(), Json::Number(Number::from(0))),
        ])),
        ResponseUsage::Usage(usage) => project::usage_json(usage),
        ResponseUsage::Mapping(mapping) => Json::Object(mapping),
    }
}

// Logging._transform_usage_objects: the chat-shaped usage a native response logs; a transcription duration projects to None.
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
        let mut prompt = IndexMap::new();
        if let Some(details) = &cost.prompt_details {
            insert_number(&mut prompt, "text_tokens", details.text_tokens);
            insert_number(&mut prompt, "audio_tokens", details.audio_tokens);
            insert_number(&mut prompt, "image_tokens", details.image_tokens);
            insert_number(&mut prompt, "video_tokens", details.video_tokens);
            insert_number(
                &mut prompt,
                "web_search_requests",
                details.web_search_requests,
            );
        }
        insert_number(&mut prompt, "cached_tokens", cost.cache_read_tokens);
        let mut completion = IndexMap::new();
        if let Some(details) = &cost.completion_details {
            insert_number(&mut completion, "text_tokens", details.text_tokens);
            insert_number(&mut completion, "audio_tokens", details.audio_tokens);
            insert_number(&mut completion, "image_tokens", details.image_tokens);
            insert_number(&mut completion, "video_tokens", details.video_tokens);
            insert_number(
                &mut completion,
                "reasoning_tokens",
                details.reasoning_tokens,
            );
        }
        Self {
            prompt_tokens: cost.prompt_tokens,
            completion_tokens: cost.completion_tokens,
            total_tokens: cost.total_tokens,
            prompt_tokens_details: Some(prompt),
            completion_tokens_details: Some(completion),
            server_tool_use: None,
            cost: None,
            extras: IndexMap::new(),
        }
    }
}

fn insert_number(record: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    if let Some(number) = value {
        record.insert(name.to_owned(), Json::Number(Number::from(number)));
    }
}

mod tests;
