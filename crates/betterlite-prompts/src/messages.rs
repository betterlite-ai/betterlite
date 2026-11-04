use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};

const CONTINUE_TEXT: &str = "Please continue.";

// System messages do not establish a turn role.
#[must_use]
pub fn detect_first_expected_role(messages: &[Message]) -> Option<&'static str> {
    for message in messages {
        match message.role.as_str() {
            "system" => {}
            "assistant" => return Some("user"),
            _ => return Some("assistant"),
        }
    }
    messages.is_empty().then_some("user")
}

fn counts_for_alternation(message: &Message) -> bool {
    if message.role == "user" {
        return true;
    }
    if message.role == "assistant" {
        return !message
            .provider_specific_fields
            .get("tool_calls")
            .is_some_and(json_list_is_nonempty);
    }
    false
}

const fn json_list_is_nonempty(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::List(items) => !items.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => true,
    }
}

fn user_continue() -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(CONTINUE_TEXT.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn assistant_continue() -> Message {
    Message {
        role: "assistant".to_owned(),
        content: Some(Content::Text(CONTINUE_TEXT.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

// Clone before insertion so provider transforms never mutate shared request input.
#[must_use]
pub fn insert_user_continue_message(
    messages: &[Message],
    custom: Option<&Message>,
    ensure: bool,
) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }
    let placeholder = custom.cloned().unwrap_or_else(user_continue);
    let mut result = messages.to_vec();
    let mut insertion_points = Vec::new();
    if messages
        .first()
        .is_some_and(|message| message.role == "assistant")
    {
        insertion_points.push(0);
    }
    insertion_points
        .extend((1..messages.len()).filter(|index| assistant_needs_user(messages, *index)));
    if ensure
        && messages
            .last()
            .is_some_and(|message| message.role == "assistant")
    {
        insertion_points.push(messages.len());
    }
    for index in insertion_points.into_iter().rev() {
        result.insert(index, placeholder.clone());
    }
    result
}

fn assistant_needs_user(messages: &[Message], index: usize) -> bool {
    let Some(current) = messages.get(index) else {
        return false;
    };
    if !counts_for_alternation(current) || current.role != "assistant" {
        return false;
    }
    let previous = messages.iter().take(index).next_back();
    let directly_after_assistant = previous.is_some_and(|message| message.role == "assistant");
    let previous_counted_assistant = messages
        .iter()
        .take(index)
        .rev()
        .find(|message| counts_for_alternation(message))
        .is_some_and(|message| message.role == "assistant");
    directly_after_assistant || previous_counted_assistant
}

// Assistant placeholders only belong between adjacent counted user turns.
#[must_use]
pub fn insert_assistant_continuation(
    messages: &[Message],
    custom: Option<&Message>,
    ensure: bool,
) -> Vec<Message> {
    if !ensure || messages.len() <= 1 {
        return messages.to_vec();
    }
    let placeholder = custom.cloned().unwrap_or_else(assistant_continue);
    let mut previous_counted_user = false;
    let mut insertions = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let counted_user = counts_for_alternation(message) && message.role == "user";
        if counted_user && previous_counted_user {
            insertions.push(index);
        }
        if counts_for_alternation(message) {
            previous_counted_user = message.role == "user";
        }
    }
    messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            let inserted = insertions.contains(&index).then(|| placeholder.clone());
            [inserted, Some(message.clone())].into_iter().flatten()
        })
        .collect()
}

// Compose user and assistant insertion in LiteLLM's specified order.
#[must_use]
pub fn get_completion_messages(
    messages: &[Message],
    assistant: Option<&Message>,
    user: Option<&Message>,
    ensure: bool,
) -> Vec<Message> {
    if !ensure {
        return messages.to_vec();
    }
    let with_users = insert_user_continue_message(messages, user, ensure);
    insert_assistant_continuation(&with_users, assistant, ensure)
}

mod tests;
