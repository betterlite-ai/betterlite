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
use rust_decimal::Error as DecimalError;
use thiserror::Error;

use crate::api::responses_usage::ResponsesUsageError;
use crate::routes::CallType;
mod convert;
pub mod provider_cost;
mod tests;
pub mod tiers;

type AnthropicUsageError = UsageError<Infallible>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    #[serde(rename = "decisions_openai")]
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
    #[error("the usage object could not be read: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the Responses usage could not be converted: {0}")]
    Responses(#[source] ResponsesUsageError),
    #[error("the Anthropic usage could not be converted: {0}")]
    Anthropic(#[source] AnthropicUsageError),
    #[error("the Interactions usage could not be converted: {0}")]
    Interactions(#[source] CostUsageError),
    #[error("the usage mapping could not be read: {0}")]
    Usage(#[source] UsageParseError),
    #[error("the {field} token count is not representable")]
    Token { field: &'static str },
    #[error("the {kind} usage does not belong to a {kind} response")]
    Projection { kind: &'static str },
    #[error("the hidden parameter {field} is not usable")]
    Hidden { field: &'static str },
    #[error("the hidden parameter {field} does not hold a number: {source}")]
    HiddenNumber {
        field: &'static str,
        #[source]
        source: DecimalError,
    },
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
    if matches!(
        response.kind,
        ResponseKind::DecisionsIr | ResponseKind::DecisionsOpenAi
    ) {
        return convert::decisions_usage(&response.usage);
    }
    usage_object(&response.usage)
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
            .map_err(BillingUsageError::Usage),
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
pub const fn infer_call_type(call_type: Option<CallType>, kind: ResponseKind) -> Option<CallType> {
    if let Some(explicit) = call_type {
        return Some(explicit);
    }
    match kind {
        ResponseKind::Chat | ResponseKind::ChatStream => Some(CallType::Completion),
        ResponseKind::Embedding => Some(CallType::Embedding),
        ResponseKind::Transcription => Some(CallType::Transcription),
        ResponseKind::Speech => Some(CallType::Speech),
        ResponseKind::Rerank => Some(CallType::Rerank),
        ResponseKind::Image => Some(CallType::ImageGeneration),
        ResponseKind::TextCompletion => Some(CallType::TextCompletion),
        ResponseKind::SendMessage => Some(CallType::SendMessage),
        ResponseKind::DecisionsIr
        | ResponseKind::DecisionsOpenAi
        | ResponseKind::Responses
        | ResponseKind::Other => None,
    }
}
pub fn transcription_has_tokens(usage: Option<&Usage>) -> Result<bool, BillingUsageError> {
    let Some(counted) = usage else {
        return Ok(false);
    };
    if let Some(details) = counted.prompt_tokens_details.as_ref()
        && (detail_count(details, "audio_tokens")? > 0 || detail_count(details, "text_tokens")? > 0)
    {
        return Ok(true);
    }
    Ok(counted.prompt_tokens > 0 || counted.completion_tokens > 0)
}
fn detail_count(
    details: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<i64, BillingUsageError> {
    match details.get(field) {
        Some(Json::Number(number)) => number.as_i64().ok_or(BillingUsageError::Token { field }),
        Some(Json::Null) | None => Ok(0),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(BillingUsageError::Token { field })
        }
    }
}
