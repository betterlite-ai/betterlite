use async_trait::async_trait;
use betterlite_prompts::message_content::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// LiteLLM falls back to litellm.token_counter when a reply carries no token counts
#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, messages: &[Message]) -> Option<u64>;
    async fn count_text(&self, text: &str) -> Option<u64>;
}
