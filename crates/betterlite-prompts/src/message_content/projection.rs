use super::{ContentOnlyMessage, Message, convert_content_list_to_str};

#[must_use]
pub fn to_content_only_messages(messages: &[Message]) -> Vec<ContentOnlyMessage> {
    let mut converted = Vec::new();
    for message in messages {
        let role = match message.role.as_str() {
            "user" | "tool" | "function" => "user",
            "assistant" => "assistant",
            _ => continue,
        };
        converted.push(ContentOnlyMessage {
            role: role.to_owned(),
            content: convert_content_list_to_str(message),
        });
    }
    converted
}

mod tests;
