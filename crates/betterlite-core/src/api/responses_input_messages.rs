#![expect(
    clippy::disallowed_types,
    reason = "the assembled chat messages are the heterogeneous JSON records LiteLLM's TypedDicts forward: each input item type contributes its own fields, so a message leaves as serialized JSON"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod dispatch;
pub mod error;
pub mod item;
pub mod reasoning;
pub mod thinking;
pub mod web_search;

mod tests;

use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_input_content::instructions_to_system_message;
use crate::api::responses_input_messages::dispatch::append_items;
use crate::api::responses_input_messages::error::InputMessagesError;

#[derive(Debug, Clone, Deserialize)]
pub struct InputRequest {
    // the Responses item union is open and provider-defined, so an item stays JSON until its type picks the converter
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub instructions: Option<String>,
}

pub(crate) struct Assembly<'context> {
    pub(crate) replay_reasoning: bool,
    pub(crate) limits: ApiLimits,
    pub(crate) cache: &'context DualLocalCache,
}

pub fn input_messages(
    request: &InputRequest,
    replay_reasoning: bool,
    limits: ApiLimits,
    cache: &DualLocalCache,
) -> Result<Vec<Value>, InputMessagesError> {
    let mut messages = Vec::new();
    if let Some(instructions) = request
        .instructions
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        messages.push(instructions_to_system_message(Some(instructions)));
    }
    let assembly = Assembly {
        replay_reasoning,
        limits,
        cache,
    };
    match request.input.as_ref() {
        None | Some(Json::Null) => {}
        Some(Json::Text(text)) => {
            messages.push(content_message("user", Value::String(text.clone())));
        }
        Some(Json::List(items)) => append_items(&mut messages, items, &assembly)?,
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            return Err(InputMessagesError::InputShape);
        }
    }
    Ok(messages)
}

pub(crate) fn content_message(role: &str, content: Value) -> Value {
    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String(role.to_owned()));
    message.insert("content".to_owned(), content);
    Value::Object(message)
}
