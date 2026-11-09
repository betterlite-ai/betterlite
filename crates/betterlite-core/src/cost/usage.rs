use std::convert::Infallible;

use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_anthropic::usage::error::UsageError;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::UsageError as UsageParseError;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_usage::error::CostUsageError;
use betterlite_models::cost_usage::transcription::{
    TranscriptionUsage, transform_transcription_usage,
};
use betterlite_models::decisions::Json;
use betterlite_models::decisions::openai::DecisionUsage;
use betterlite_models::decisions::systemone::DecisionsUsage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::responses_usage::ResponsesUsageError;

mod convert;
pub mod provider_cost;
mod tests;
pub mod tiers;

type AnthropicUsageError = UsageError<Infallible>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    Chat,
    ChatStream,
    Embedding,
    Transcription,
    Speech,
    Rerank,
    Image,
    TextCompletion,
    SendMessage,
    DecisionsIr,
    DecisionsOpenAi,
    Responses,
    Other,
}

pub enum BillingUsage<'usage> {
    Absent,
    Null,
    Chat(&'usage Usage),
    Response(&'usage ResponseApiUsage),
    Transcription(&'usage TranscriptionUsage),
    DecisionsIr(&'usage DecisionsUsage),
    DecisionsOpenAi(&'usage DecisionUsage),
    Interactions(&'usage IndexMap<String, Json>),
    Mapping(&'usage IndexMap<String, Json>),
    TypedRecord(&'usage IndexMap<String, Json>),
}

pub struct BillingResponse<'usage> {
    pub kind: ResponseKind,
    pub usage: BillingUsage<'usage>,
    pub service_tier: Option<&'usage str>,
    pub hidden_params: Option<&'usage IndexMap<String, Json>>,
}

#[derive(Debug, Error)]
pub enum BillingUsageError {
    #[error("the usage object could not be read: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Responses usage could not be converted: {source}")]
    Responses {
        #[source]
        source: ResponsesUsageError,
    },
    #[error("the Anthropic usage could not be converted: {source}")]
    Anthropic {
        #[source]
        source: AnthropicUsageError,
    },
    #[error("the Interactions usage could not be converted: {source}")]
    Interactions {
        #[source]
        source: CostUsageError,
    },
    #[error("the usage mapping could not be read: {source}")]
    Usage {
        #[source]
        source: UsageParseError,
    },
    #[error("the {field} count does not fit a signed 64-bit integer")]
    TokenCount { field: &'static str },
    #[error("the {field} token total overflowed")]
    TokenOverflow { field: &'static str },
    #[error("the {kind} usage does not belong to a {kind} response")]
    Projection { kind: &'static str },
    #[error("the hidden response cost is not a number")]
    ResponseCostNotNumber,
    #[error("the hidden parameter {field} is not an object")]
    HiddenParameter { field: &'static str },
}

// The source only asks for Anthropic usage here with reasoning_content None, so the counter never runs.
struct NoReasoningCounter;

impl ReasoningTokenCounter for NoReasoningCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub fn get_usage_object(
    response: &BillingResponse<'_>,
) -> Result<Option<Usage>, BillingUsageError> {
    match response.kind {
        ResponseKind::DecisionsIr | ResponseKind::DecisionsOpenAi => {
            decisions_usage(&response.usage)
        }
        ResponseKind::Chat
        | ResponseKind::ChatStream
        | ResponseKind::Embedding
        | ResponseKind::Transcription
        | ResponseKind::Speech
        | ResponseKind::Rerank
        | ResponseKind::Image
        | ResponseKind::TextCompletion
        | ResponseKind::SendMessage
        | ResponseKind::Responses
        | ResponseKind::Other => usage_object(&response.usage),
    }
}

fn decisions_usage(usage: &BillingUsage<'_>) -> Result<Option<Usage>, BillingUsageError> {
    match usage {
        BillingUsage::Absent | BillingUsage::Null => Ok(None),
        BillingUsage::DecisionsIr(found) => convert::decisions::from_ir_usage(found).map(Some),
        BillingUsage::DecisionsOpenAi(found) => {
            convert::decisions::from_openai_usage(found).map(Some)
        }
        BillingUsage::Chat(_)
        | BillingUsage::Response(_)
        | BillingUsage::Transcription(_)
        | BillingUsage::Interactions(_)
        | BillingUsage::Mapping(_)
        | BillingUsage::TypedRecord(_) => Err(BillingUsageError::Projection { kind: "decision" }),
    }
}

fn usage_object(usage: &BillingUsage<'_>) -> Result<Option<Usage>, BillingUsageError> {
    match usage {
        BillingUsage::Absent | BillingUsage::Null => Ok(None),
        BillingUsage::Chat(found) => Ok(Some((*found).clone())),
        BillingUsage::Response(found) => convert::responses_usage(found).map(Some),
        BillingUsage::Transcription(found) => {
            Ok(transform_transcription_usage(found).map(|cost| Usage::from(&cost)))
        }
        BillingUsage::Interactions(found) => convert::interactions_usage(found).map(Some),
        BillingUsage::Mapping(found) => convert::mapping_usage(found),
        BillingUsage::TypedRecord(found) => usage_from_mapping(found)
            .map(Some)
            .map_err(|source| BillingUsageError::Usage { source }),
        BillingUsage::DecisionsIr(_) | BillingUsage::DecisionsOpenAi(_) => {
            Err(BillingUsageError::Projection { kind: "decisions" })
        }
    }
}

#[must_use]
pub const fn is_known_usage_object(usage: &BillingUsage<'_>) -> bool {
    matches!(
        usage,
        BillingUsage::Chat(_)
            | BillingUsage::Response(_)
            | BillingUsage::Transcription(_)
            | BillingUsage::DecisionsIr(_)
            | BillingUsage::DecisionsOpenAi(_)
    )
}

#[must_use]
pub const fn infer_call_type(call_type: Option<&str>, kind: ResponseKind) -> Option<&str> {
    if let Some(explicit) = call_type {
        return Some(explicit);
    }
    match kind {
        ResponseKind::Chat | ResponseKind::ChatStream => Some("completion"),
        ResponseKind::Embedding => Some("embedding"),
        ResponseKind::Transcription => Some("transcription"),
        ResponseKind::Speech => Some("speech"),
        ResponseKind::Rerank => Some("rerank"),
        ResponseKind::Image => Some("image_generation"),
        ResponseKind::TextCompletion => Some("text_completion"),
        ResponseKind::SendMessage => Some("send_message"),
        ResponseKind::DecisionsIr
        | ResponseKind::DecisionsOpenAi
        | ResponseKind::Responses
        | ResponseKind::Other => None,
    }
}

#[must_use]
pub fn transcription_has_tokens(usage: Option<&Usage>) -> bool {
    let Some(counted) = usage else {
        return false;
    };
    if let Some(details) = counted.prompt_tokens_details.as_ref() {
        let audio = detail_count(details, "audio_tokens");
        let text = detail_count(details, "text_tokens");
        if audio > 0 || text > 0 {
            return true;
        }
    }
    counted.prompt_tokens > 0 || counted.completion_tokens > 0
}

fn detail_count(details: &IndexMap<String, Json>, name: &str) -> i64 {
    match details.get(name) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}
