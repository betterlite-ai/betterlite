use betterlite_prompts::message_content::{Message, convert_content_list_to_str};

use crate::purview_graph_client::tool_call_extraction::tool_call_arguments;

// Messages join with a blank line so a pattern never runs across two messages.
#[must_use]
pub fn prompt_text_for_dlp(messages: &[Message]) -> Option<String> {
    let mut parts = Vec::new();
    for message in messages {
        let mut segments = Vec::new();
        let content = convert_content_list_to_str(message);
        if !content.trim().is_empty() {
            segments.push(content.trim().to_owned());
        }
        segments.extend(tool_call_arguments(&message.provider_specific_fields));
        let combined = segments.join("\n");
        if !combined.trim().is_empty() {
            parts.push(combined.trim().to_owned());
        }
    }
    Some(parts.join("\n\n")).filter(|text| !text.is_empty())
}

mod tests;
