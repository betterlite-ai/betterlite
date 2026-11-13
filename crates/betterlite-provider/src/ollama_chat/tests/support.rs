use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use indexmap::IndexMap;

use crate::ollama_chat::ids::IdSource;
use crate::ollama_chat::usage::TokenCounter;

pub(super) struct SequenceIds {
    next: AtomicU32,
}

impl SequenceIds {
    pub(super) fn new() -> Self {
        Self {
            next: AtomicU32::new(0),
        }
    }
}

impl IdSource for SequenceIds {
    fn next_id(&self) -> String {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        format!("id-{id}")
    }
}

pub(super) struct WordsCounter;

fn count_words(text: &str) -> u64 {
    u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
}

#[async_trait]
impl TokenCounter for WordsCounter {
    async fn count_messages(&self, messages: &[Message]) -> Option<u64> {
        let mut total = 0_u64;
        for message in messages {
            total = total.saturating_add(count_words(&convert_content_list_to_str(message)));
        }
        Some(total)
    }

    async fn count_text(&self, text: &str) -> Option<u64> {
        Some(count_words(text))
    }
}

pub(super) struct FailingCounter;

#[async_trait]
impl TokenCounter for FailingCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Option<u64> {
        None
    }

    async fn count_text(&self, _text: &str) -> Option<u64> {
        None
    }
}

pub(super) fn messages(json: &str) -> Vec<Message> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn options(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).unwrap()
}
