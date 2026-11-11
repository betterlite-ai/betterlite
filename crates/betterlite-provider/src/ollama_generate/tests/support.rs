use std::future::Future;
use std::future::ready;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;

use crate::ollama_chat::ids::IdSource;
use crate::ollama_chat::usage::TokenCounter;
use crate::ollama_generate::embed::TokenEncoder;

pub(super) const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
pub(super) const JPEG_BASE64: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==";

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
        format!("id-{}", self.next.fetch_add(1, Ordering::Relaxed))
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

pub(super) struct GoneCounter;

#[async_trait]
impl TokenCounter for GoneCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Option<u64> {
        None
    }

    async fn count_text(&self, _text: &str) -> Option<u64> {
        None
    }
}

pub(super) struct WordsEncoder;

#[async_trait]
impl TokenEncoder for WordsEncoder {
    async fn encode(&self, text: &str) -> Result<Vec<u32>, CountError> {
        let count = text.split_whitespace().count();
        Ok(vec![1_u32; count])
    }
}

pub(super) struct FailingEncoder;

#[async_trait]
impl TokenEncoder for FailingEncoder {
    async fn encode(&self, _text: &str) -> Result<Vec<u32>, CountError> {
        Err(CountError::Overflow)
    }
}

pub(super) struct MapFetcher;

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        if url.contains("png") {
            ready(Ok(png_data_url()))
        } else {
            ready(Err(ImageFetchError::Rejected {
                url: url.to_owned(),
            }))
        }
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(
            urls.iter()
                .map(|_url| Ok(png_data_url()))
                .collect::<Result<Vec<String>, ImageFetchError>>(),
        )
    }
}

pub(super) fn png_data_url() -> String {
    format!("data:image/png;base64,{PNG_BASE64}")
}

pub(super) fn jpeg_data_url() -> String {
    format!("data:image/jpeg;base64,{JPEG_BASE64}")
}

pub(super) fn bmp_base64() -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&58_u32.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 255, 0]);
    STANDARD.encode(bytes)
}

pub(super) fn bmp_data_url() -> String {
    format!("data:image/bmp;base64,{}", bmp_base64())
}

pub(super) fn text_message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

pub(super) fn messages(json: &str) -> Vec<Message> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn options(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn request_data(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).unwrap()
}
