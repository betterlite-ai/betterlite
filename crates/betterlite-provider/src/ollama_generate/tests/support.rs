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
        ready(if url.contains("png") {
            Ok(png_data_url())
        } else {
            Err(ImageFetchError::Rejected {
                url: url.to_owned(),
            })
        })
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        if let Some(url) = urls.iter().find(|url| !url.contains("png")) {
            return ready(Err(ImageFetchError::Rejected {
                url: url.to_owned(),
            }));
        }
        let values: Vec<String> = urls.iter().map(|_url| png_data_url()).collect();
        ready(Ok(values))
    }
}

pub(super) fn png_data_url() -> String {
    format!("data:image/png;base64,{PNG_BASE64}")
}

pub(super) fn jpeg_data_url() -> String {
    format!("data:image/jpeg;base64,{JPEG_BASE64}")
}

fn bmp_bytes(width: u32, height: u32) -> Vec<u8> {
    let row_bytes = width.saturating_mul(3);
    let stride = row_bytes.next_multiple_of(4);
    let pixel_bytes = stride.saturating_mul(height);
    let file_size = pixel_bytes.saturating_add(54);
    let length = usize::try_from(file_size).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&file_size.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&pixel_bytes.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.resize(length, 0);
    bytes
}

pub(super) fn bmp_base64() -> String {
    STANDARD.encode(bmp_bytes(1, 1))
}

pub(super) fn bmp_data_url() -> String {
    format!("data:image/bmp;base64,{}", bmp_base64())
}

pub(super) fn oversized_bmp_data_url() -> String {
    format!(
        "data:image/bmp;base64,{}",
        STANDARD.encode(bmp_bytes(8193, 1))
    )
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
