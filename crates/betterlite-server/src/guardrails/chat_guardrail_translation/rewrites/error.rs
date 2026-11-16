use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use betterlite_core::passthrough_guardrail::errors::PassthroughPayloadError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;
use betterlite_streaming::streamed_response_assembly::AssemblyError;
use thiserror::Error;

use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error(transparent)]
    Apply(#[from] ApplyError),
    #[error("the guardrail returned tool call {task_index}, which is not an object")]
    ToolCallNotObject { task_index: usize },
    #[error("the guardrail returned a function {field} that is not text")]
    FieldNotText { field: &'static str },
    #[error("{field} is not a list")]
    NotAList { field: &'static str },
    #[error("{field} entry {index} is not an object")]
    NotAnObject { field: &'static str, index: usize },
    #[error("the stream has no choice to rebuild")]
    NoStreamChoices,
    #[error("the rebuilt response for choice {index} has no choice")]
    NoRebuiltChoice { index: i64 },
    #[error("the stream did not assemble into a chat response")]
    NotAChatResponse,
    #[error("the response cannot be written as guardrail data: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Payload(#[from] PassthroughPayloadError),
    #[error("the stream chunks could not be assembled: {source}")]
    Assembly {
        #[source]
        source: Box<AssemblyError>,
    },
}

impl From<RewriteError> for GuardrailFailure {
    fn from(error: RewriteError) -> Self {
        Self::Other(error.to_string())
    }
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Unappliable(#[from] UnappliableRequestRewrite),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Apply(#[from] ApplyError),
}

#[derive(Debug, Error)]
pub enum EndedStreamError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Undeliverable(#[from] UndeliverableStreamRewrite),
}
