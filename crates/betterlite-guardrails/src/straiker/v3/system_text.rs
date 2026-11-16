use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker::v3::PREAMBLE_ROLES;
use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::messages::{conversation_messages, message_text};

// The preamble, wherever the API puts it: Anthropic's `system`, the Responses API's `instructions`, or the leading system or developer message of an OpenAI chat body.
pub fn system_text(request_body: &Object) -> Result<Option<String>, V3Error> {
    match request_body.get("system") {
        Some(Json::Text(system)) => return Ok(Some(system.clone())),
        Some(Json::Null) | None => {}
        Some(blocks) => {
            return serde_json::to_string(blocks)
                .map(Some)
                .map_err(V3Error::Serialize);
        }
    }
    if let Some(Json::Text(instructions)) = request_body.get("instructions") {
        return Ok(Some(instructions.clone()));
    }
    let preamble = conversation_messages(request_body)
        .into_iter()
        .find(|message| {
            message
                .get("role")
                .and_then(Json::as_str)
                .is_some_and(|role| PREAMBLE_ROLES.contains(&role))
        });
    Ok(preamble.map(message_text))
}

mod tests;
