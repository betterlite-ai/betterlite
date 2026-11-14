use betterlite_models::call_helpers::finish_reason::{FinishReason, FinishReasonMapping};
use betterlite_openai::chat::{ChatMessage, MessageContent};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;

use super::error::PredictionError;
use super::records::{Prediction, ReplyMeta, UsageMetadata, source_string};

// B195 counts tokens, the server's clock supplies created, and B206 maps the finish reason
pub struct ProjectionServices<'a> {
    pub now: u64,
    pub encoding: &'a dyn LocalEncoding,
    pub finish_reason: fn(&str) -> FinishReasonMapping,
}

pub trait LocalEncoding {
    fn encode(&self, text: &str) -> Result<usize, EngineError>;
}

impl LocalEncoding for Engine {
    fn encode(&self, text: &str) -> Result<usize, EngineError> {
        Engine::encode(self, text).map(|ids| ids.len())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LegacyMessage {
    Copied(ChatMessage),
    Content(String),
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegacyResult {
    pub message: LegacyMessage,
    pub created: u64,
    pub model: String,
    pub usage: LegacyUsage,
    pub finish_reason: Option<FinishReason>,
}

pub struct ProjectionInput<'a> {
    pub prediction: Option<&'a Prediction>,
    pub meta: Option<&'a ReplyMeta>,
    pub model_is_language: bool,
    pub prompt: &'a str,
    pub model: &'a str,
}

pub fn project(
    input: &ProjectionInput<'_>,
    services: &ProjectionServices<'_>,
) -> Result<LegacyResult, PredictionError> {
    let message = message_of(input.prediction)?;
    let (finish_reason, usage) = usage_of(input, &message, services)?;
    Ok(LegacyResult {
        message,
        created: services.now,
        model: input.model.to_owned(),
        usage,
        finish_reason,
    })
}

fn message_of(prediction: Option<&Prediction>) -> Result<LegacyMessage, PredictionError> {
    match prediction {
        None => Ok(LegacyMessage::Content(String::from("None"))),
        Some(Prediction::Message(message)) => Ok(LegacyMessage::Copied(message.clone())),
        Some(Prediction::Value(value)) => {
            let text = source_string(value);
            if text.is_empty() {
                Ok(LegacyMessage::Unchanged)
            } else {
                Ok(LegacyMessage::Content(text))
            }
        }
    }
}

fn usage_of(
    input: &ProjectionInput<'_>,
    message: &LegacyMessage,
    services: &ProjectionServices<'_>,
) -> Result<(Option<FinishReason>, LegacyUsage), PredictionError> {
    if input.model_is_language
        && let Some(meta) = input.meta
    {
        return language_usage(meta, services);
    }
    if let Some(usage) = input
        .meta
        .and_then(|meta| meta.usage_metadata.as_ref())
        .filter(|usage| usage.prompt_token_count.is_some())
    {
        return server_usage(usage);
    }
    local_usage(input.prompt, message, services).map(|usage| (None, usage))
}

fn language_usage(
    meta: &ReplyMeta,
    services: &ProjectionServices<'_>,
) -> Result<(Option<FinishReason>, LegacyUsage), PredictionError> {
    let candidates = meta
        .candidates
        .as_ref()
        .ok_or(PredictionError::MissingField {
            field: "candidates",
        })?;
    let first = candidates.first().ok_or(PredictionError::MissingField {
        field: "candidates",
    })?;
    let raw = first
        .finish_reason
        .as_deref()
        .ok_or(PredictionError::MissingField {
            field: "finishReason",
        })?;
    let usage = meta
        .usage_metadata
        .as_ref()
        .ok_or(PredictionError::MissingUsage {
            field: "usageMetadata",
        })?;
    let prompt_tokens = usage
        .prompt_token_count
        .ok_or(PredictionError::MissingUsage {
            field: "promptTokenCount",
        })?;
    let completion_tokens = usage
        .candidates_token_count
        .ok_or(PredictionError::MissingUsage {
            field: "candidatesTokenCount",
        })?;
    let total_tokens = usage
        .total_token_count
        .ok_or(PredictionError::MissingUsage {
            field: "totalTokenCount",
        })?;
    Ok((
        Some((services.finish_reason)(raw).reason),
        LegacyUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
    ))
}

fn server_usage(
    usage: &UsageMetadata,
) -> Result<(Option<FinishReason>, LegacyUsage), PredictionError> {
    let prompt_tokens = usage
        .prompt_token_count
        .ok_or(PredictionError::MissingUsage {
            field: "promptTokenCount",
        })?;
    let completion_tokens = usage
        .candidates_token_count
        .ok_or(PredictionError::MissingUsage {
            field: "candidatesTokenCount",
        })?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(PredictionError::UsageOverflow)?;
    Ok((
        None,
        LegacyUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
    ))
}

fn local_usage(
    prompt: &str,
    message: &LegacyMessage,
    services: &ProjectionServices<'_>,
) -> Result<LegacyUsage, PredictionError> {
    let prompt_tokens = encode(services.encoding, prompt)?;
    let completion_tokens = encode(services.encoding, &content_of(message)?)?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(PredictionError::UsageOverflow)?;
    Ok(LegacyUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

fn encode(encoding: &dyn LocalEncoding, text: &str) -> Result<i64, PredictionError> {
    let count = encoding
        .encode(text)
        .map_err(|source| PredictionError::Encoding { source })?;
    i64::try_from(count).map_err(|_error| PredictionError::UsageOverflow)
}

fn content_of(message: &LegacyMessage) -> Result<String, PredictionError> {
    match message {
        LegacyMessage::Copied(message) => match &message.content {
            None => Ok(String::new()),
            Some(MessageContent::Text(text)) => Ok(text.clone()),
            Some(MessageContent::Parts(_parts)) => Err(PredictionError::NonTextContent),
        },
        LegacyMessage::Content(text) => Ok(text.clone()),
        LegacyMessage::Unchanged => Ok(String::new()),
    }
}
