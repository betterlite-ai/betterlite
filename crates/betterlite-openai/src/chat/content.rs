use crate::chat::response::ChatResponse;

// LiteLLM also concatenates the legacy function_call field here; betterlite does not model it (B218 deviations).
#[must_use]
pub fn from_model_response(response: &ChatResponse) -> String {
    let mut content = String::new();
    for choice in &response.choices {
        if let Some(text) = &choice.message.content {
            content.push_str(text);
        }
        if let Some(tool_calls) = &choice.message.tool_calls {
            for tool_call in tool_calls {
                content.push_str(&serde_json::to_string(tool_call).unwrap_or_default());
            }
        }
    }
    content
}

mod tests;
