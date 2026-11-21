use crate::agentcore_response::transform::RawResponse;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};
use crate::invoke_agent_reply::decode_agent_trace;
use crate::invoke_agent_reply::error::{BedrockFailure, InvokeAgentReplyError};
use crate::invoke_agent_reply::extract::{ExtractError, UsageInfo, response_content, usage_info};
use crate::invoke_agent_reply::wire::{Choice, Message, ModelResponse, Usage};

// transform_response: decode the whole buffered stream, then overwrite the supplied response's choices, model and usage.
pub fn transform_invoke_agent_response(
    raw: &RawResponse,
    response: &mut ModelResponse,
) -> Result<(), InvokeAgentReplyError> {
    match apply(raw, response) {
        Ok(()) => Ok(()),
        Err(error) => Err(bedrock_failure(raw, &error)),
    }
}

fn apply(raw: &RawResponse, response: &mut ModelResponse) -> Result<(), InvokeAgentReplyError> {
    let events = decode_agent_trace(&raw.body)?;
    let content = response_content(&events)?;
    let usage = usage_info(&events)?;
    build(content, &usage, response)
}

fn build(
    content: String,
    usage: &UsageInfo,
    response: &mut ModelResponse,
) -> Result<(), InvokeAgentReplyError> {
    let total = usage
        .input_tokens
        .checked_add(usage.output_tokens)
        .ok_or(ExtractError::Overflow)?;
    response.choices = vec![Choice {
        finish_reason: "stop".to_owned(),
        index: 0,
        message: Message {
            content: Some(content),
            role: "assistant".to_owned(),
        },
    }];
    // _build_model_response reads usage_info.get("model", model); the key is always set, so the supplied model never wins.
    response.model = usage.model.clone();
    response.usage = Some(Usage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        total_tokens: total,
    });
    Ok(())
}

// Any uncaught failure becomes one B682 BedrockError carrying the raw response's status, never a status of its own.
fn bedrock_failure(raw: &RawResponse, error: &InvokeAgentReplyError) -> InvokeAgentReplyError {
    tracing::error!("Error processing Bedrock Invoke Agent response: {error}");
    let input = ErrorInput {
        status_code: raw.status_code,
        message: format!("Error processing response: {error}"),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(raw.headers.clone())),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(bedrock) => InvokeAgentReplyError::Bedrock {
            status: raw.status_code,
            error: BedrockFailure {
                cause: Box::new(bedrock),
            },
        },
        Err(source) => InvokeAgentReplyError::Header(source),
    }
}
