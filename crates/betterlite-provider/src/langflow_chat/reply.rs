use async_trait::async_trait;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;

use super::error::LangFlowError;

pub mod decode;

const ASSISTANT_ROLE: &str = "assistant";
const STOP_FINISH_REASON: &str = "stop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChoice {
    pub index: u32,
    pub finish_reason: String,
    pub message: AssistantMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangFlowChatResponse {
    pub model: String,
    pub choice: ChatChoice,
    pub usage: Option<Usage>,
}

// LiteLLM calls token_counter(model=model, ...) twice, so the requested model picks the encoding
#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<u64, CountError>;
    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<u64, CountError>;
}

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub status: u16,
    pub body: &'request [u8],
    pub messages: &'request [Message],
}

pub async fn transform_response(
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> Result<LangFlowChatResponse, LangFlowError> {
    let content = decode::decode_content(input.body, input.status)?;
    let usage = count_usage(counter, input.model, input.messages, &content).await;
    Ok(LangFlowChatResponse {
        model: input.model.to_owned(),
        choice: ChatChoice {
            index: 0,
            finish_reason: String::from(STOP_FINISH_REASON),
            message: AssistantMessage {
                role: String::from(ASSISTANT_ROLE),
                content,
            },
        },
        usage,
    })
}

async fn count_usage(
    counter: &dyn TokenCounter,
    model: &str,
    messages: &[Message],
    content: &str,
) -> Option<Usage> {
    let prompt_tokens = match counter.count_messages(model, messages).await {
        Ok(count) => count,
        Err(error) => return counter_failed(&error),
    };
    let completion_tokens = match counter.count_response_tokens(model, content).await {
        Ok(count) => count,
        Err(error) => return counter_failed(&error),
    };
    let total_tokens = match prompt_tokens.checked_add(completion_tokens) {
        Some(total) => total,
        None => return counter_failed(&CountError::Overflow),
    };
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

fn counter_failed(error: &CountError) -> Option<Usage> {
    tracing::warn!("Failed to calculate token usage: {error}");
    None
}
