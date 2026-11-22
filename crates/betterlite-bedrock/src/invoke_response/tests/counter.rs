#![cfg(test)]

use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use crate::pegasus::counter::TokenCounter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CounterCall {
    Messages,
    Text(String),
}

pub(super) struct FixtureCounter {
    text: Option<i64>,
    messages: Option<i64>,
    calls: Mutex<Vec<CounterCall>>,
}

impl FixtureCounter {
    pub(super) fn counting(text: i64, messages: i64) -> Self {
        Self {
            text: Some(text),
            messages: Some(messages),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            text: None,
            messages: None,
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
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(CounterCall::Messages);
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

pub(super) struct FixedIds {
    pub(super) values: Mutex<Vec<Uuid>>,
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            return Uuid::nil();
        }
        values.remove(0)
    }
}
