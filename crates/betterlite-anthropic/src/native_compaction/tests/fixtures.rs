#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::native_compaction::anthropic::ModelInfoLookup;
use crate::native_compaction::dispatch::{ChatCall, CompactionRouter};
use crate::native_compaction::protocol::Mapping;

#[derive(Clone, Copy, Debug, Error)]
#[error("fake failure")]
pub(crate) struct FakeFailure;

pub(crate) fn mapping(text: &str) -> Mapping {
    serde_json::from_str(text).unwrap()
}

pub(crate) struct FakeLookup {
    answer: Result<Option<bool>, FakeFailure>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeLookup {
    pub(crate) fn answering(answer: Result<Option<bool>, FakeFailure>) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn recorded(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelInfoLookup for FakeLookup {
    type Error = FakeFailure;

    fn supports_anthropic_compaction(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> impl Future<Output = Result<Option<bool>, FakeFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), custom_llm_provider.to_owned()));
        ready(self.answer)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatRecord {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) stream: bool,
    pub(crate) options: IndexMap<String, Json>,
}

pub(crate) struct FakeRouter {
    reply: Result<Mapping, FakeFailure>,
    messages_calls: Mutex<Vec<Mapping>>,
    chat_calls: Mutex<Vec<ChatRecord>>,
}

impl FakeRouter {
    pub(crate) fn replying(reply: Result<Mapping, FakeFailure>) -> Self {
        Self {
            reply,
            messages_calls: Mutex::new(Vec::new()),
            chat_calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn messages_recorded(&self) -> Vec<Mapping> {
        self.messages_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn chat_recorded(&self) -> Vec<ChatRecord> {
        self.chat_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompactionRouter for FakeRouter {
    type Error = FakeFailure;

    fn aanthropic_messages(
        &self,
        payload: &Mapping,
    ) -> impl Future<Output = Result<Mapping, FakeFailure>> + Send {
        self.messages_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        ready(self.reply.clone())
    }

    fn acompletion(
        &self,
        call: &ChatCall<'_>,
    ) -> impl Future<Output = Result<Mapping, FakeFailure>> + Send {
        self.chat_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ChatRecord {
                model: call.model.to_owned(),
                messages: call.messages.to_vec(),
                stream: call.stream,
                options: call
                    .options
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).clone()))
                    .collect(),
            });
        ready(self.reply.clone())
    }
}
