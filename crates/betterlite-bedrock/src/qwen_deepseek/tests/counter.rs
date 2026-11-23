#![cfg(test)]

use std::convert::Infallible;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

use crate::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use crate::pegasus::counter::TokenCounter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CounterCall {
    Messages(String),
    Text(String),
}

pub(super) struct FixtureCounter {
    text: Option<i64>,
    messages: Option<i64>,
    calls: Mutex<Vec<CounterCall>>,
}

impl FixtureCounter {
    pub(super) const fn counting(prompt: i64, completion: i64) -> Self {
        Self {
            text: Some(completion),
            messages: Some(prompt),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<CounterCall> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

#[async_trait]
impl TokenCounter for FixtureCounter {
    async fn count_messages(&self, messages: &[Message]) -> Result<i64, CountError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(CounterCall::Messages(format!("{messages:?}")));
        }
        self.messages.ok_or(CountError::MissingInput)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(CounterCall::Text(text.to_owned()));
        }
        self.text.ok_or(CountError::MissingInput)
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
