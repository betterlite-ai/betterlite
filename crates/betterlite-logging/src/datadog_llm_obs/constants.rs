use betterlite_models::decisions::Json;

pub const MAX_PARSED_TOOL_ARGUMENT_CHARS: usize = 256 * 1024;

pub const SAFE_REDACTED_MESSAGE_ROLES: [&str; 8] = [
    "agent",
    "assistant",
    "developer",
    "function",
    "model",
    "system",
    "tool",
    "user",
];

#[must_use]
pub fn safe_message_role(role: Option<&Json>) -> String {
    match role {
        Some(Json::Text(name)) if SAFE_REDACTED_MESSAGE_ROLES.contains(&name.as_str()) => {
            name.clone()
        }
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => String::new(),
    }
}
