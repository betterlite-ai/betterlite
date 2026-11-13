use betterlite_models::decisions::Json;

use super::literal::literal_json;
use super::messages::to_dd_messages;
use super::records::DdMessage;
use super::values::as_object;

#[rustfmt::skip]
const LLM_OBS_CALL_TYPES: [&str; 9] = [
    "completion", "acompletion", "text_completion", "atext_completion", "generate_content",
    "agenerate_content", "generate_content_stream", "agenerate_content_stream", "anthropic_messages",
];

#[must_use]
pub fn response_messages(response: Option<&Json>, call_type: Option<&str>) -> Vec<DdMessage> {
    let Some(value) = response else {
        return Vec::new();
    };
    let decoded = match value {
        Json::Null => return Vec::new(),
        Json::Text(text) => {
            let Some(parsed) =
                literal_json(text).or_else(|| serde_json::from_str::<Json>(text).ok())
            else {
                return Vec::new();
            };
            parsed
        }
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => value.clone(),
    };
    let Some(fields) = as_object(&decoded) else {
        return Vec::new();
    };
    if !call_type.is_some_and(|name| LLM_OBS_CALL_TYPES.contains(&name)) {
        return Vec::new();
    }
    let Some(message) = fields
        .get("choices")
        .and_then(|choices| match choices {
            Json::List(items) => items.first(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
        })
        .and_then(as_object)
        .and_then(|choice| choice.get("message"))
    else {
        return Vec::new();
    };
    to_dd_messages(Some(&Json::List(vec![message.clone()])))
}
