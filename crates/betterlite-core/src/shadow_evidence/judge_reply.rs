use betterlite_models::decisions::Json;

use super::chat_text;
use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_tool_namespaces::values::has_value;

mod tests;

#[must_use]
pub fn shape(response: &Json) -> String {
    let Some(message) = chat_text::message(response) else {
        return "unreadable judge reply".to_owned();
    };
    let body = chat_text::field(message, "content")
        .filter(|value| has_value(value))
        .map_or_else(
            || "no content".to_owned(),
            |content| format!("{} chars", string_value(content).chars().count()),
        );
    let served = chat_text::field(response, "model")
        .filter(|value| has_value(value))
        .map_or_else(|| "unknown".to_owned(), string_value);
    format!(
        "finish_reason={}, content={body}, model={served}",
        chat_text::finish_reason(response)
    )
}
