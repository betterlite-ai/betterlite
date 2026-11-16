use std::fmt;
use std::marker::PhantomData;

use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use betterlite_openai::chat::response::{
    AssistantMessage, ChatResponse, ChatUsage, MessageProviderFields,
};
use serde::de::value::MapAccessDeserializer;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::api::fal::vision_chat::error::FalAIError;
use crate::api::fal::vision_chat::token_count::TokenCount;

// a record read only from a JSON object: a derived Deserialize would also take a JSON array field by field
#[derive(Debug)]
struct JsonObject<T>(T);

struct JsonObjectVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for JsonObjectVisitor<T> {
    type Value = JsonObject<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        T::deserialize(MapAccessDeserializer::new(map)).map(JsonObject)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for JsonObject<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(JsonObjectVisitor(PhantomData))
    }
}

// fields Fal sends beyond these are ignored, as LiteLLM's model does
#[derive(Debug, Deserialize)]
struct RawUsage {
    input_tokens: TokenCount,
    output_tokens: TokenCount,
}

#[derive(Debug, Deserialize)]
struct RawReply {
    output: String,
    usage_info: JsonObject<RawUsage>,
    reasoning: Option<String>,
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalChatReply {
    pub output: String,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// the record's assistant message has no reasoning_content field, so the reasoning travels beside it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalChatCompletion {
    pub response: ChatResponse,
    pub reasoning_content: Option<String>,
}

pub fn decode_fal_chat(headers: &ImageHeaders, body: &[u8]) -> Result<FalChatReply, FalAIError> {
    let JsonObject(raw): JsonObject<RawReply> =
        serde_json::from_slice(body).map_err(|source| FalAIError::UnexpectedResponse {
            text: replaced_text(body),
            headers: headers.clone(),
            source,
        })?;
    let JsonObject(usage) = raw.usage_info;
    let TokenCount(prompt_tokens) = usage.input_tokens;
    let TokenCount(completion_tokens) = usage.output_tokens;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or_else(|| FalAIError::UsageOverflow {
            text: replaced_text(body),
            headers: headers.clone(),
        })?;
    Ok(FalChatReply {
        output: raw.output,
        reasoning: raw.reasoning,
        finish_reason: raw.finish_reason,
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

// the first choice is rewritten and every other field of the response keeps what it had
pub fn transform_response(
    model: &str,
    headers: &ImageHeaders,
    body: &[u8],
    mut response: ChatResponse,
    created: u64,
) -> Result<FalChatCompletion, FalAIError> {
    let reply = decode_fal_chat(headers, body)?;
    let choice = response.choices.first_mut().ok_or(FalAIError::NoChoices)?;
    choice.message = AssistantMessage {
        role: "assistant".to_owned(),
        content: Some(reply.output),
        tool_calls: None,
        annotations: None,
        audio: None,
        refusal: None,
        provider_specific_fields: MessageProviderFields::default(),
    };
    let finish_reason = reply.finish_reason.as_deref().unwrap_or("stop");
    choice.finish_reason = Some(map_finish_reason(finish_reason).reason.as_str().to_owned());
    response.created = created;
    model.clone_into(&mut response.model);
    response.usage = ChatUsage {
        prompt_tokens: reply.prompt_tokens,
        completion_tokens: reply.completion_tokens,
        total_tokens: reply.total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    Ok(FalChatCompletion {
        response,
        reasoning_content: reply.reasoning,
    })
}

// httpx replaces each invalid sequence with one U+FFFD, which clippy.toml keeps String::from_utf8_lossy from doing
fn replaced_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
