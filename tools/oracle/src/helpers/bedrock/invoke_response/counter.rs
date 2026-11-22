use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_bedrock::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde_json::{Value, json};

use betterlite_oracle_runner::helpers::{HelperCall, invoke};

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

// The oracle answer for a fallback count is the token_count helper, so no tokenizer runs twice here.
pub(super) struct TokenizerCounter {
    pub(super) messages: Json,
}

#[async_trait]
impl TokenCounter for TokenizerCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        count(json!({
            "function": "token_counter",
            "model": "",
            "messages": self.messages,
        }))
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        count(json!({
            "function": "token_counter",
            "model": "",
            "text": text,
            "count_response_tokens": true,
        }))
    }
}

// The token_count helper starts its own runtime, so the call runs off the runtime this decode drives.
fn count(input: Value) -> Result<i64, CountError> {
    let groups = [betterlite_oracle_tokenizer::GROUP];
    let counted = std::thread::spawn(move || {
        invoke(
            &groups,
            &HelperCall {
                operation: String::from("token_count"),
                input,
            },
        )
    })
    .join()
    .map_err(|_panic| CountError::MissingInput)?;
    let Some(value) = counted.value else {
        return Err(CountError::MissingInput);
    };
    serde_json::from_value(value).map_err(|_error| CountError::MissingInput)
}
