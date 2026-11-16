use async_trait::async_trait;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use serde::{Deserialize, Serialize};

use super::messages::ASSISTANT_ROLE;

pub const COUNT_MODEL: &str = "gpt-3.5-turbo";
pub const STOP_FINISH_REASON: &str = "stop";
pub const THREAD_ID_KEY: &str = "thread_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplyMessage {
    pub role: String,
    pub content: String,
    pub annotations: Option<Vec<Object>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentChoice {
    pub index: u32,
    pub finish_reason: String,
    pub message: ReplyMessage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentReply {
    pub model: Option<String>,
    pub choices: Vec<AgentChoice>,
    // None covers a missing _hidden_params and one that is None; LiteLLM stamps a fresh dict for both
    pub hidden_params: Option<Object>,
    pub usage: Option<Usage>,
}

pub struct TokenCountRequest<'request> {
    pub model: &'request str,
    pub messages: Option<&'request [Object]>,
    pub text: Option<&'request str>,
    pub count_response_tokens: bool,
}

// LiteLLM reaches the tokenizer through litellm.utils.token_counter, which this seam stands in for
#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError>;
}

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub content: &'request str,
    pub thread_id: &'request str,
    pub messages: &'request [Object],
    pub annotations: Option<Vec<Object>>,
}

pub async fn build_model_response(
    existing: AgentReply,
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> AgentReply {
    let AgentReply {
        hidden_params,
        usage,
        ..
    } = existing;
    let mut reply = AgentReply {
        model: Some(input.model.to_owned()),
        choices: vec![AgentChoice {
            index: 0,
            finish_reason: String::from(STOP_FINISH_REASON),
            message: ReplyMessage {
                role: String::from(ASSISTANT_ROLE),
                content: input.content.to_owned(),
                annotations: input
                    .annotations
                    .clone()
                    .filter(|values| !values.is_empty()),
            },
        }],
        hidden_params: Some(stamped(hidden_params, input.thread_id)),
        usage,
    };
    if let Some(counted) = counted_usage(counter, input).await {
        reply.usage = Some(counted);
    }
    reply
}

async fn counted_usage(counter: &dyn TokenCounter, input: &ReplyInput<'_>) -> Option<Usage> {
    let prompt = TokenCountRequest {
        model: COUNT_MODEL,
        messages: Some(input.messages),
        text: None,
        count_response_tokens: false,
    };
    let prompt_tokens = match counter.count_request(&prompt).await {
        Ok(tokens) => tokens,
        Err(error) => return counter_failed(&error),
    };
    let reply = TokenCountRequest {
        model: COUNT_MODEL,
        messages: None,
        text: Some(input.content),
        count_response_tokens: true,
    };
    let completion_tokens = match counter.count_request(&reply).await {
        Ok(tokens) => tokens,
        Err(error) => return counter_failed(&error),
    };
    let Some(total_tokens) = prompt_tokens.checked_add(completion_tokens) else {
        return counter_failed(&CountError::Overflow);
    };
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

// a failed count keeps whatever usage the response already carried rather than synthesizing zero
fn counter_failed(error: &CountError) -> Option<Usage> {
    tracing::warn!("Failed to calculate token usage: {error}");
    None
}

fn stamped(current: Option<Object>, thread_id: &str) -> Object {
    let mut params = current.unwrap_or_default();
    params.insert(
        String::from(THREAD_ID_KEY),
        Json::Text(thread_id.to_owned()),
    );
    params
}
