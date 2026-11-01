use crate::arguments::json::Object;
use crate::message_content::{Content, Message, convert_content_list_to_str};

const USER_ROLE: &str = "user";

#[must_use]
pub fn get_last_user_message(messages: &[Message]) -> Option<String> {
    let user_run = trailing_role_run(messages, USER_ROLE);
    if user_run.is_empty() {
        return None;
    }
    let mut prompt = String::new();
    for message in user_run {
        prompt.push_str(&convert_content_list_to_str(message));
        prompt.push('\n');
    }
    let trimmed = prompt.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[must_use]
pub fn set_last_user_message(messages: &[Message], content: &str) -> Vec<Message> {
    let keep = messages
        .len()
        .saturating_sub(trailing_role_run(messages, USER_ROLE).len());
    let mut updated = messages.get(..keep).unwrap_or(&[]).to_vec();
    updated.push(Message {
        role: USER_ROLE.to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    });
    updated
}

fn trailing_role_run<'messages>(
    messages: &'messages [Message],
    role: &str,
) -> &'messages [Message] {
    let trailing_count = messages
        .iter()
        .rev()
        .take_while(|message| message.role == role)
        .count();
    let split = messages.len().saturating_sub(trailing_count);
    messages.get(split..).unwrap_or(&[])
}

mod tests;
