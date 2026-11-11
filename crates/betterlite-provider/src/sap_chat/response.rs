use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::Json;
use regex::Regex;
use serde::Deserialize;

use super::error::SapChatError;
use crate::sap_stream::Mode;

const FENCE_PATTERN: &str = r"(?s)^```(?:json)?\s*\n?(.*?)\n?```$";
const JSON_FORMATS: [&str; 2] = ["json_object", "json_schema"];

pub struct PostCall<'call> {
    pub input: &'call [Json],
    pub api_key: Option<&'call str>,
    pub original_response: &'call str,
    pub request_data: &'call Json,
}

pub trait PostCallLogger: Send + Sync {
    fn post_call(&self, call: PostCall<'_>);
}

pub struct ResponseInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub api_key: Option<&'request str>,
    pub raw_response: &'request str,
    pub request_data: &'request Json,
    pub response_format: Option<&'request Json>,
}

#[derive(Deserialize)]
struct ChatReply {
    final_result: ChatResponse,
}

pub fn transform_response(
    input: &ResponseInput<'_>,
    logger: &dyn PostCallLogger,
) -> Result<ChatResponse, SapChatError> {
    logger.post_call(PostCall {
        input: input.messages,
        api_key: input.api_key,
        original_response: input.raw_response,
        request_data: input.request_data,
    });
    let mut response = serde_json::from_str::<ChatReply>(input.raw_response)
        .map_err(SapChatError::Response)?
        .final_result;
    strip_markdown_json(&mut response, input.model, input.response_format);
    Ok(response)
}

// LiteLLM's iterator choice only changes the mode B5952's line stream runs in
#[must_use]
pub const fn get_model_response_iterator(sync_stream: bool) -> Mode {
    if sync_stream { Mode::Sync } else { Mode::Async }
}

fn strip_markdown_json(response: &mut ChatResponse, model: &str, response_format: Option<&Json>) {
    let kind = response_format
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str);
    if !model.starts_with("anthropic") || !kind.is_some_and(|name| JSON_FORMATS.contains(&name)) {
        return;
    }
    let Ok(pattern) = Regex::new(FENCE_PATTERN) else {
        return;
    };
    for choice in &mut response.choices {
        let Some(content) = choice.message.content.as_deref() else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        if let Some(captures) = pattern.captures(content.trim())
            && let Some(stripped) = captures.get(1)
        {
            choice.message.content = Some(stripped.as_str().trim().to_owned());
        }
    }
}
