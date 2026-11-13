use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::masking::masked_headers;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use super::MistralChatError;
use super::wire::MistralContentBlock;
use crate::compatible_wire::{
    CompatibleReply, LogInput, ReplyOptions, decode_compatible_chat_reply,
};
use crate::openai_base_config::Headers;

pub struct ResponseReply<'reply> {
    pub status: u16,
    pub headers: &'reply Headers,
    pub body: &'reply [u8],
}

pub struct MistralChatReply {
    pub value: Value,
}

pub fn transform_response(
    reply: &ResponseReply<'_>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    input: LogInput,
    options: &ReplyOptions<'_>,
) -> Result<CompatibleReply, MistralChatError> {
    let call = PostCall {
        original_response: std::str::from_utf8(reply.body)
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
        input: input.messages,
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(input.data),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    post_call(state, &call, env);
    let headers: IndexMap<String, Json> = reply
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    state.model_call_details.insert(
        String::from("response_headers"),
        Json::Object(masked_headers(&headers, false)),
    );
    let decoded = decode_mistral_chat(reply.body)?;
    let body = serde_json::to_vec(&decoded.value)
        .map_err(|source| MistralChatError::ReplyEncode { source })?;
    Ok(decode_compatible_chat_reply(&body, options)?)
}

pub fn decode_mistral_chat(bytes: &[u8]) -> Result<MistralChatReply, MistralChatError> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|source| MistralChatError::ReplyJson { source })?;
    handle_empty_content_response(&mut value);
    handle_content_list_to_str_conversion(&mut value)?;
    Ok(MistralChatReply { value })
}

pub fn handle_empty_content_response(reply: &mut Value) {
    let Some(choices) = reply.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) else {
            continue;
        };
        if message
            .get("content")
            .is_some_and(|content| content.as_str() == Some(""))
        {
            message.insert(String::from("content"), Value::Null);
        }
    }
}

pub fn handle_content_list_to_str_conversion(reply: &mut Value) -> Result<(), MistralChatError> {
    let Some(choices) = reply.get_mut("choices").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) else {
            continue;
        };
        let Value::Array(blocks) = message.get("content").cloned().unwrap_or(Value::Null) else {
            continue;
        };
        if blocks.is_empty() {
            continue;
        }
        let mut text_content = String::new();
        let mut thinking_content = String::new();
        for block in blocks {
            let decoded: MistralContentBlock = serde_json::from_value(block)
                .map_err(|source| MistralChatError::ReplyBlock { source })?;
            match decoded.kind.as_str() {
                "thinking" => thinking_content = decoded.thinking_text("\n"),
                "text" => text_content = decoded.text.unwrap_or_default(),
                _ => {}
            }
        }
        message.insert(String::from("content"), Value::String(text_content));
        if !thinking_content.is_empty() {
            message.insert(
                String::from("reasoning_content"),
                Value::String(thinking_content),
            );
        }
    }
    Ok(())
}
