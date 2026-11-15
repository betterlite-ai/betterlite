use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use super::chat_text;

mod tests;

// LiteLLM accepts only these two output item types as tool calls, matched exactly
const TOOL_CALL_TYPES: [&str; 2] = ["function_call", "custom_tool_call"];

#[must_use]
pub fn final_text(response: &Json) -> String {
    let Some(response_text) = serde_json::to_string(response).ok() else {
        return String::new();
    };
    let Ok(parsed) = serde_json::from_str::<ResponsesApiResponse>(&response_text) else {
        return String::new();
    };
    let Some(items_text) = serde_json::to_string(&parsed.output).ok() else {
        return String::new();
    };
    let Ok(items) = serde_json::from_str::<Vec<Json>>(&items_text) else {
        return String::new();
    };
    let calls: Vec<&Json> = items.iter().filter(|item| is_tool_call(item)).collect();
    let prose = parsed.output_text();
    if calls.is_empty() {
        return prose;
    }
    let serialized = chat_text::serialize_invocations(&calls);
    if prose.is_empty() {
        return serialized;
    }
    format!("{prose} {serialized}").trim().to_owned()
}

fn is_tool_call(item: &Json) -> bool {
    matches!(
        chat_text::field(item, "type"),
        Some(Json::Text(kind)) if TOOL_CALL_TYPES.contains(&kind.as_str())
    )
}
