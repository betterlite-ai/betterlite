use std::error::Error as StdError;

use betterlite_core::rules::RuleError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::{
    MidStreamFallbackDetails, MidStreamStatus, TimeoutDetails, TimeoutStatus,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_redact::secret::Redactor;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use thiserror::Error;

use super::ChatStream;
use super::config::ChatStreamConfig;
use super::ports::StreamSourceError;

// What broke the stream, before betterlite's taxonomy names it.
#[derive(Debug, Error)]
pub enum ChatStreamCause {
    #[error(transparent)]
    Source(#[from] StreamSourceError),
    #[error(transparent)]
    Chunk(#[from] ProviderChunkError),
    #[error(transparent)]
    Rules(#[from] RuleError),
    #[error(transparent)]
    Shape(#[from] StreamShapeError),
    #[error("{message}")]
    Duration { message: String },
}

#[derive(Debug, Error)]
#[error("{cause}")]
pub struct ChatStreamError {
    pub class: ErrorClass,
    #[source]
    pub cause: ChatStreamCause,
}

// exception_type()'s mapping, kept apart from the fallback wrapper it feeds.
pub(super) fn mapped(
    cause: &ChatStreamCause,
    config: &ChatStreamConfig,
    redactor: &Redactor,
) -> ErrorClass {
    match cause {
        ChatStreamCause::Duration { message } => timeout_class(message, config),
        ChatStreamCause::Rules(error) => error.classify(),
        ChatStreamCause::Source(error) => error.class.clone(),
        ChatStreamCause::Chunk(error) => chunk_class(error, config, redactor),
        ChatStreamCause::Shape(shape) => shape_class(shape, config, redactor),
    }
}

// A failure that never reaches handle_stream_fallback_error, such as the blocking duration check.
pub(super) fn direct<S>(cause: ChatStreamCause, stream: &ChatStream<'_, S>) -> ChatStreamError {
    ChatStreamError {
        class: mapped(&cause, &stream.config, stream.ports.redactor),
        cause,
    }
}

// handle_stream_fallback_error: a non-429 4xx raises as mapped, everything else becomes a fallback.
pub(super) fn wrap<S>(
    class: ErrorClass,
    cause: ChatStreamCause,
    stream: &ChatStream<'_, S>,
) -> ChatStreamError {
    let status = class.metadata().status_code;
    if (400..500).contains(&status) && status != 429 {
        return ChatStreamError { class, cause };
    }
    let provider = stream
        .config
        .provider
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or("anthropic");
    let context = context(cause.to_string(), &stream.config.model, provider);
    let details = MidStreamFallbackDetails {
        context,
        original_status: MidStreamStatus(status),
        generated_content: stream.shaper.response_uptil_now.clone(),
        is_pre_first_chunk: !stream.shaper.progress.sent_first_chunk,
    };
    ChatStreamError {
        class: ErrorClass::MidStreamFallback(Box::new(details)),
        cause,
    }
}

fn timeout_class(message: &str, config: &ChatStreamConfig) -> ErrorClass {
    let provider = config.provider.as_deref().unwrap_or("");
    let details = TimeoutDetails {
        context: context(message.to_owned(), &config.model, provider),
        original_status: TimeoutStatus::default(),
    };
    ErrorClass::Timeout(Box::new(details))
}

// A repeating model raises litellm.InternalServerError with an empty model and provider.
fn chunk_class(
    error: &ProviderChunkError,
    config: &ChatStreamConfig,
    redactor: &Redactor,
) -> ErrorClass {
    if matches!(
        error,
        ProviderChunkError::Shape {
            source: StreamShapeError::Repetition { .. }
        }
    ) {
        let details = context(error.to_string(), "", "");
        return ErrorClass::InternalServer(Box::new(details));
    }
    raw_class(error, error.class(), config, redactor)
}

fn shape_class(
    shape: &StreamShapeError,
    config: &ChatStreamConfig,
    redactor: &Redactor,
) -> ErrorClass {
    match shape {
        StreamShapeError::Repetition { .. } => {
            let details = context(shape.to_string(), "", "");
            ErrorClass::InternalServer(Box::new(details))
        }
        StreamShapeError::Arguments { .. } => raw_class(shape, "ValueError", config, redactor),
    }
}

// exception_type() over an exception betterlite only knows as text and a Python class name.
fn raw_class(
    error: &dyn StdError,
    exception_type: &str,
    config: &ChatStreamConfig,
    redactor: &Redactor,
) -> ErrorClass {
    let raw = RawException {
        model: config.model.clone(),
        custom_llm_provider: config.provider.clone().unwrap_or_default(),
        error_str: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: exception_type.to_owned(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

pub(super) fn context(message: String, model: &str, provider: &str) -> ErrorContext {
    ErrorContext {
        message,
        model: Some(model.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
