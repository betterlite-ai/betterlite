use std::collections::BTreeMap;
use std::fmt;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};
use serde::{Deserialize, Serialize};

use crate::agentcore_response::counter::TokenCounter;
use crate::agentcore_response::error::{AgentCoreResponseError, BedrockFailure};
use crate::agentcore_response::reply::decode_agentcore_reply;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub async fn transform_agentcore_response<C>(
    model: &str,
    raw: &RawResponse,
    messages: &[Message],
    response: &mut ChatResponse,
    counter: &C,
) -> Result<(), AgentCoreResponseError>
where
    C: TokenCounter + Sync,
{
    if let Err(error) = apply(model, raw, messages, response, counter).await {
        return Err(bedrock_failure(raw, &error));
    }
    Ok(())
}

fn bedrock_failure(raw: &RawResponse, error: &AgentCoreResponseError) -> AgentCoreResponseError {
    tracing::error!("Error processing Bedrock AgentCore response: {error}");
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
        Ok(bedrock) => AgentCoreResponseError::Bedrock {
            status: raw.status_code,
            error: BedrockFailure {
                cause: Box::new(bedrock),
            },
        },
        Err(source) => AgentCoreResponseError::Header(source),
    }
}

async fn apply<C>(
    model: &str,
    raw: &RawResponse,
    messages: &[Message],
    response: &mut ChatResponse,
    counter: &C,
) -> Result<(), AgentCoreResponseError>
where
    C: TokenCounter + Sync,
{
    let reply = decode_agentcore_reply(&raw.body, content_type(&raw.headers).as_deref())?;
    response.choices = vec![ChatChoice {
        index: 0,
        message: AssistantMessage {
            role: "assistant".to_owned(),
            content: Some(reply.content.clone()),
            tool_calls: None,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields { refusal: None },
        },
        finish_reason: Some("stop".to_owned()),
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }];
    response.model = String::from(model);
    if let Some(usage) = reply.usage {
        response.usage = usage.to_chat_usage();
    } else {
        tracing::debug!("No usage data from AgentCore - calculating tokens");
        if let Some(usage) = calculate_usage(model, messages, &reply.content, counter).await {
            response.usage = usage;
        }
    }
    Ok(())
}

async fn calculate_usage<C>(
    model: &str,
    messages: &[Message],
    content: &str,
    counter: &C,
) -> Option<ChatUsage>
where
    C: TokenCounter + Sync,
{
    let prompt_request = usage_request(model, Some(messages), None);
    let prompt = match counter.count(&prompt_request).await {
        Ok(count) => count,
        Err(error) => return counter_failed(error),
    };
    let text = TextInput::One(content.to_owned());
    let completion_request = usage_request(model, None, Some(&text));
    let completion = match counter.count(&completion_request).await {
        Ok(count) => count,
        Err(error) => return counter_failed(error),
    };
    let prompt_tokens = u64::try_from(prompt).ok()?;
    let completion_tokens = u64::try_from(completion).ok()?;
    let total_tokens = u64::try_from(prompt.checked_add(completion)?).ok()?;
    tracing::debug!(
        "Calculated usage - prompt: {prompt_tokens}, completion: {completion_tokens}, total: {total_tokens}"
    );
    Some(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    })
}

const fn usage_request<'request>(
    model: &'request str,
    messages: Option<&'request [Message]>,
    text: Option<&'request TextInput>,
) -> TokenRequest<'request> {
    TokenRequest {
        model,
        custom_tokenizer: None,
        text,
        messages,
        count_response_tokens: Some(text.is_some()),
        tools: None,
        tool_choice: None,
    }
}

fn counter_failed<E>(error: E) -> Option<ChatUsage>
where
    E: fmt::Display,
{
    tracing::warn!("Failed to calculate token usage: {error}");
    None
}

fn content_type(headers: &[(String, String)]) -> Option<String> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}
