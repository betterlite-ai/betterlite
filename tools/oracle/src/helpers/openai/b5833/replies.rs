use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_openai::compatible_wire::ReplyOptions;
use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::reply::transform_response;

use super::Call;

pub(super) fn reply_one(call: &Call) -> Result<Value, GroqChatError> {
    let options = ReplyOptions {
        json_mode: call.json_mode.unwrap_or(false),
        provider: None,
        base_model: None,
        identifier: Uuid::nil,
        clock: i64::default,
    };
    let body = call.reply.clone().unwrap_or(Value::Null);
    let converted = transform_response(body.to_string().as_bytes(), &options)?;
    let actions = converted.browser_actions;
    Ok(json!({
        "service_tier": converted.reply.response.service_tier,
        "web_search_requests": actions.map(|counts| counts.web_search_requests),
        "browser_open_requests": actions.map(|counts| counts.browser_open_requests),
    }))
}
