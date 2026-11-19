use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::response::wire::{Choice, ResponseEnvelope};
use betterlite_anthropic::usage::AnthropicUsage;
use serde::Serialize;

use crate::anthropic_dispatch::AnthropicResult;
use crate::anthropic_dispatch::gateway::{error, frames};
use crate::operation::{OperationError, OperationResult};
use crate::prepared::{PreparedRequest, Upstream};

// The client's view of a Messages reply: everything LiteLLM sends, without the internal _hidden_params.
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicReply {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Option<AnthropicUsage>,
}

impl AnthropicReply {
    #[must_use]
    pub(crate) fn new(envelope: &ResponseEnvelope, requested: Option<&str>) -> Self {
        let model = requested
            .map(str::to_owned)
            .or_else(|| envelope.model.clone())
            .unwrap_or_default();
        Self {
            id: envelope.id.clone(),
            object: envelope.object.clone(),
            created: envelope.created,
            model,
            system_fingerprint: envelope.system_fingerprint.clone(),
            choices: envelope.choices.clone(),
            usage: envelope.usage.clone(),
        }
    }
}

// The Messages outcome as the client view; a legacy reply never comes back through this adapter.
pub(crate) fn decode(
    prepared: &PreparedRequest,
    upstream: Upstream,
) -> Result<OperationResult, OperationError> {
    let PreparedRequest::Anthropic(prepared) = prepared else {
        return Err(error::unsupported());
    };
    let Upstream::Anthropic(upstream) = upstream else {
        return Err(error::unsupported());
    };
    let requested = prepared.chat.request.model.as_deref();
    match upstream.outcome {
        ChatOutcome::Reply(envelope) => Ok(OperationResult::Anthropic(AnthropicResult::Reply(
            Box::new(AnthropicReply::new(&envelope, requested)),
        ))),
        ChatOutcome::Stream(stream) => Ok(OperationResult::Anthropic(AnthropicResult::Stream(
            frames::frames(*stream, &prepared.chat.request, &prepared.chat.model),
        ))),
        ChatOutcome::Embedding => Err(error::unsupported()),
    }
}
