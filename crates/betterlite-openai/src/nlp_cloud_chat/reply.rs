use std::collections::BTreeMap;
use std::str;

use serde_json::Value;

use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::nlp_cloud_chat::error::NlpCloudError;
use crate::openai_base_config::Object;

pub struct ReplyInput<'body> {
    pub status_code: u16,
    pub body: &'body [u8],
    pub model: &'body str,
    pub created: u64,
    pub id: String,
}

pub fn decode_reply(input: ReplyInput<'_>) -> Result<ChatResponse, NlpCloudError> {
    let response = reply_object(&input)?;
    let Some(generated_text) = response.get("generated_text").and_then(Value::as_str) else {
        return Err(malformed(input.status_code, input.body));
    };
    let usage = reply_usage(&response, input.body)?;
    Ok(chat_response(input, generated_text, usage))
}

fn reply_object(input: &ReplyInput<'_>) -> Result<Object, NlpCloudError> {
    let parsed: Value = match serde_json::from_slice(input.body) {
        Ok(value) => value,
        Err(_source) => return Err(malformed(input.status_code, input.body)),
    };
    let Value::Object(response) = parsed else {
        return Err(malformed(input.status_code, input.body));
    };
    if let Some(error) = response.get("error") {
        return Err(NlpCloudError {
            status_code: input.status_code,
            message: error_text(error),
            headers: None,
        });
    }
    Ok(response)
}

fn reply_usage(response: &Object, body: &[u8]) -> Result<ChatUsage, NlpCloudError> {
    let prompt_tokens = response
        .get("nb_input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed(500, body))?;
    let completion_tokens = response
        .get("nb_generated_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed(500, body))?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or_else(|| malformed(500, body))?;
    Ok(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    })
}

fn chat_response(input: ReplyInput<'_>, generated_text: &str, usage: ChatUsage) -> ChatResponse {
    ChatResponse {
        id: input.id,
        object: "chat.completion".to_owned(),
        created: input.created,
        model: input.model.to_owned(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: "assistant".to_owned(),
                content: (!generated_text.is_empty()).then(|| generated_text.to_owned()),
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: Some("stop".to_owned()),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage,
        system_fingerprint: None,
        service_tier: None,
    }
}

fn error_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

fn malformed(status_code: u16, body: &[u8]) -> NlpCloudError {
    NlpCloudError {
        status_code,
        message: body_text(body),
        headers: None,
    }
}

fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| "the reply body is not valid UTF-8".to_owned(),
        str::to_owned,
    )
}
