use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_bedrock::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

// The reply cases carry usage headers, so the base conversion never falls back to a tokenizer.
pub(super) struct ZeroCounter;

#[async_trait]
impl TokenCounter for ZeroCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }
}

pub(super) struct NoReasoning;

impl AnthropicReasoningCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

impl ConverseReasoningCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}
