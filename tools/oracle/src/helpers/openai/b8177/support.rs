use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_openai::gigachat_chat::config::{GigachatBearer, GigachatImageUploader};
use betterlite_openai::gigachat_chat::error::GigachatTokenError;
use betterlite_openai::gigachat_utils::IdSource;
use betterlite_openai::openai_base_config::Object;
use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, Usage};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

// LiteLLM derives every id from uuid4, so both sides pin the same one
pub(crate) const FIXED_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(FIXED_UUID)
    }
}

pub(crate) struct TokenBearer {
    pub(crate) token: SecretString,
    pub(crate) seen: Mutex<Vec<Option<String>>>,
}

impl TokenBearer {
    pub(crate) fn new(token: &str) -> Self {
        Self {
            token: SecretString::from(token),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn credentials(&self) -> Option<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .flatten()
    }
}

#[async_trait]
impl GigachatBearer for TokenBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigachatTokenError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(credentials.map(|value| value.expose_secret().to_owned()));
        Ok(self.token.clone())
    }
}

// the reply step reads no token and uploads nothing
pub(crate) struct UnusedBearer;

#[async_trait]
impl GigachatBearer for UnusedBearer {
    async fn access_token(
        &self,
        _credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigachatTokenError> {
        Err(GigachatTokenError {
            status_code: 500,
            message: String::from("this helper step needs no token"),
        })
    }
}

pub(crate) struct Uploader {
    pub(crate) ids: BTreeMap<String, String>,
}

#[async_trait]
impl GigachatImageUploader for Uploader {
    async fn upload(&self, image_url: &str) -> Option<String> {
        self.ids.get(image_url).cloned()
    }
}

pub(crate) struct UnusedUploader;

#[async_trait]
impl GigachatImageUploader for UnusedUploader {
    async fn upload(&self, _image_url: &str) -> Option<String> {
        None
    }
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

pub(crate) fn success(entries: Vec<(&str, Value)>) -> Value {
    let mut object = Object::new();
    object.insert(String::from("ok"), Value::Bool(true));
    for (key, value) in entries {
        object.insert(String::from(key), value);
    }
    Value::Object(object)
}

pub(crate) fn failure(status_code: u16) -> Value {
    Value::Object(Object::from_iter([
        (String::from("ok"), Value::Bool(false)),
        (String::from("status_code"), Value::from(status_code)),
    ]))
}

pub(crate) fn failure_without_status() -> Value {
    Value::Object(Object::from_iter([(
        String::from("ok"),
        Value::Bool(false),
    )]))
}

pub(crate) fn chat_reply_value(response: &ChatResponse) -> Value {
    let mut choices = Vec::with_capacity(response.choices.len());
    for choice in &response.choices {
        let tool_calls = choice
            .message
            .tool_calls
            .as_ref()
            .map_or(Value::Null, |calls| {
                Value::Array(calls.iter().map(tool_call_value).collect())
            });
        choices.push(Value::Object(Object::from_iter([
            (String::from("index"), Value::from(choice.index)),
            (
                String::from("role"),
                Value::String(choice.message.role.clone()),
            ),
            (
                String::from("content"),
                choice
                    .message
                    .content
                    .as_ref()
                    .map_or(Value::Null, |text| Value::String(text.clone())),
            ),
            (String::from("tool_calls"), tool_calls),
            (
                String::from("finish_reason"),
                choice
                    .finish_reason
                    .as_ref()
                    .map_or(Value::Null, |reason| Value::String(reason.clone())),
            ),
        ])));
    }
    success(vec![
        ("id", Value::from(response.id.clone())),
        ("created", Value::from(response.created)),
        ("model", Value::from(response.model.clone())),
        ("choices", Value::Array(choices)),
        ("usage", chat_usage_value(&response.usage)),
    ])
}

fn tool_call_value(call: &ToolCall) -> Value {
    Value::Object(Object::from_iter([
        (String::from("id"), Value::String(call.id.clone())),
        (String::from("type"), Value::String(call.kind.clone())),
        (
            String::from("function"),
            Value::Object(Object::from_iter([
                (
                    String::from("name"),
                    Value::String(call.function.name.clone()),
                ),
                (
                    String::from("arguments"),
                    // LiteLLM writes the arguments as a JSON string; the compared value is the JSON it holds
                    serde_json::from_str(&call.function.arguments)
                        .unwrap_or_else(|_| Value::String(call.function.arguments.clone())),
                ),
            ])),
        ),
    ]))
}

fn chat_usage_value(usage: &ChatUsage) -> Value {
    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.get("cached_tokens"))
        .copied()
        .flatten();
    Value::Object(Object::from_iter([
        (
            String::from("prompt_tokens"),
            Value::from(usage.prompt_tokens),
        ),
        (
            String::from("completion_tokens"),
            Value::from(usage.completion_tokens),
        ),
        (
            String::from("total_tokens"),
            Value::from(usage.total_tokens),
        ),
        (
            String::from("cached_tokens"),
            cached.map_or(Value::Null, Value::from),
        ),
    ]))
}

pub(crate) fn stream_chunk_value(chunk: &GenericChunk) -> Value {
    Value::Object(Object::from_iter([
        (String::from("text"), Value::String(chunk.text.clone())),
        (
            String::from("tool_use"),
            chunk.tool_use.as_ref().map_or(Value::Null, |tool_use| {
                serde_json::to_value(tool_use).unwrap_or(Value::Null)
            }),
        ),
        (String::from("is_finished"), Value::Bool(chunk.is_finished)),
        (
            String::from("finish_reason"),
            Value::String(chunk.finish_reason.clone()),
        ),
        (
            String::from("usage"),
            chunk.usage.as_ref().map_or(Value::Null, stream_usage_value),
        ),
        (
            String::from("index"),
            chunk.index.map_or(Value::Null, Value::from),
        ),
    ]))
}

fn stream_usage_value(usage: &Usage) -> Value {
    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    Value::Object(Object::from_iter([
        (
            String::from("prompt_tokens"),
            Value::from(usage.prompt_tokens),
        ),
        (
            String::from("completion_tokens"),
            Value::from(usage.completion_tokens),
        ),
        (
            String::from("total_tokens"),
            Value::from(usage.total_tokens),
        ),
        (
            String::from("cached_tokens"),
            cached.map_or(Value::Null, Value::from),
        ),
    ]))
}
