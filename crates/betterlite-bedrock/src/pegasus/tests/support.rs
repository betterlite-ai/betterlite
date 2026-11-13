use std::future::Future;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

use crate::pegasus::counter::TokenCounter;
use crate::pegasus::reply::ReplyContext;
use crate::pegasus::response::{PegasusChoice, PegasusMessage, PegasusResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CounterCall {
    Messages,
    Text(String),
}

pub(super) struct FixtureCounter {
    pub(super) text: Option<i64>,
    pub(super) messages: Option<i64>,
    pub(super) calls: Mutex<Vec<CounterCall>>,
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

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(_failure) => panic!("the test runtime could not start"),
    }
}

pub(super) fn header_counts(input: &str, output: &str) -> Vec<(String, String)> {
    vec![
        (
            "x-amzn-bedrock-input-token-count".to_owned(),
            input.to_owned(),
        ),
        (
            "x-amzn-bedrock-output-token-count".to_owned(),
            output.to_owned(),
        ),
    ]
}

pub(super) fn response(choices: Vec<PegasusChoice>) -> PegasusResponse {
    PegasusResponse {
        id: "chatcmpl-pegasus".to_owned(),
        choices,
        created: 0,
        model: String::new(),
        usage: Default::default(),
    }
}

pub(super) fn choice(index: i64, content: Option<&str>, finish_reason: &str) -> PegasusChoice {
    PegasusChoice {
        index,
        message: PegasusMessage {
            role: "assistant".to_owned(),
            content: content.map(str::to_owned),
            tool_calls: None,
        },
        finish_reason: Some(finish_reason.to_owned()),
    }
}

pub(super) fn single_choice() -> PegasusResponse {
    response(vec![choice(0, None, "stop")])
}

pub(super) fn context<'request>(
    headers: &'request [(String, String)],
    messages: &'request [Message],
    now: f64,
) -> ReplyContext<'request> {
    ReplyContext {
        status: 200,
        headers,
        model: "twelvelabs.pegasus-1-2-v1:0",
        messages,
        now,
    }
}
