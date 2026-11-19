use std::collections::BTreeMap;

use http::HeaderMap;
use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::stream::chunk::{Choice, Chunk, Delta};

use super::tokens::{PromptCountError, PromptTokens};

// the injected clock, id and prompt count of one recovery, so the response never reads the wall clock or the tokenizer itself
pub struct Recovery<'recovery> {
    pub clock: fn() -> u64,
    pub next_id: fn() -> Uuid,
    pub tokens: &'recovery dyn PromptTokens,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RecoveredBody {
    Completion(ChatResponse),
    Chunk(Chunk),
}

#[derive(Debug)]
pub struct OutputLimitResponse {
    pub headers: HeaderMap,
    pub body: RecoveredBody,
}

// LiteLLM's build_output_token_limit_response: the length-truncated answer the provider sends for slightly larger budgets
pub async fn build_limit_response(
    data: &Map<String, Value>,
    headers: HeaderMap,
    recovery: &Recovery<'_>,
) -> Result<OutputLimitResponse, PromptCountError> {
    let model = data
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let prompt_tokens = match data.get("messages") {
        Some(Value::Array(messages)) => recovery.tokens.count(model, messages).await?,
        _ => 0,
    };
    let id = format!("chatcmpl-{}", (recovery.next_id)());
    let created = (recovery.clock)();
    let response = completion(&id, created, model, prompt_tokens);
    let body = if matches!(data.get("stream"), Some(Value::Bool(true))) {
        RecoveredBody::Chunk(chunk(response))
    } else {
        RecoveredBody::Completion(response)
    };
    Ok(OutputLimitResponse { headers, body })
}

fn completion(id: &str, created: u64, model: &str, prompt_tokens: usize) -> ChatResponse {
    let counted = u64::try_from(prompt_tokens).unwrap_or(u64::MAX);
    ChatResponse {
        id: id.to_owned(),
        object: String::from("chat.completion"),
        created,
        model: model.to_owned(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: Some(String::new()),
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: Some(String::from("length")),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        // the reply is truncated, so the completion count and the detail fields stay zero and absent
        usage: ChatUsage {
            prompt_tokens: counted,
            completion_tokens: 0,
            total_tokens: counted,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        system_fingerprint: None,
        service_tier: None,
    }
}

// LiteLLM sends no usage object on the one chunk even though the prompt was counted
fn chunk(response: ChatResponse) -> Chunk {
    let choices = response
        .choices
        .into_iter()
        .map(|choice| Choice {
            index: choice.index,
            delta: Delta {
                role: Some(choice.message.role),
                content: choice.message.content,
                refusal: choice.message.refusal,
                tool_calls: None,
                function_call: None,
                reasoning_content: None,
                thinking_blocks: None,
            },
            finish_reason: choice.finish_reason,
            logprobs: None,
        })
        .collect();
    Chunk {
        id: response.id,
        object: String::from("chat.completion.chunk"),
        created: response.created,
        model: response.model,
        choices,
        usage: None,
        system_fingerprint: response.system_fingerprint,
        service_tier: response.service_tier,
    }
}
