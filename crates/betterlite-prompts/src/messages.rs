use serde_json::{Value, json};

const USER_CONTINUE: &str = "Please continue.";
const ASSISTANT_CONTINUE: &str = "Please continue.";

// System messages do not establish a turn role.
#[must_use]
pub fn detect_first_expected_role(messages: &[Value]) -> Option<&'static str> {
    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("system") => {}
            Some("assistant") => return Some("user"),
            Some("user") => return Some("assistant"),
            Some(_) => return Some("assistant"),
            None => return Some("assistant"),
        }
    }
    if messages.is_empty() {
        Some("user")
    } else {
        None
    }
}

fn counts_for_alternation(message: &Value) -> bool {
    match message.get("role").and_then(Value::as_str) {
        Some("user") => true,
        Some("assistant") => !message.get("tool_calls").is_some_and(|calls| {
            !calls.is_null() && calls.as_array().is_none_or(|items| !items.is_empty())
        }),
        _ => false,
    }
}

// Clone before insertion so provider transforms never mutate shared request input.
#[must_use]
pub fn insert_user_continue_message(
    messages: &[Value],
    custom: Option<&Value>,
    ensure: bool,
) -> Vec<Value> {
    if messages.is_empty() {
        return Vec::new();
    }
    let placeholder = custom
        .cloned()
        .unwrap_or_else(|| json!({"role": "user", "content": USER_CONTINUE}));
    let mut result = messages.to_vec();
    let mut insertion_points = Vec::new();
    if first_role_is_assistant(messages) {
        insertion_points.push(0);
    }
    insertion_points
        .extend((1..messages.len()).filter(|index| assistant_needs_user(messages, *index)));
    if ensure
        && messages
            .last()
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            == Some("assistant")
    {
        insertion_points.push(messages.len());
    }
    for index in insertion_points.into_iter().rev() {
        result.insert(index, placeholder.clone());
    }
    result
}

fn first_role_is_assistant(messages: &[Value]) -> bool {
    messages
        .first()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        == Some("assistant")
}

fn assistant_needs_user(messages: &[Value], index: usize) -> bool {
    let Some(current) = messages.get(index) else {
        return false;
    };
    if !counts_for_alternation(current)
        || current.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return false;
    }
    let previous = messages.iter().take(index).next_back();
    let directly_after_assistant = previous
        .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("assistant"));
    let previous_counted_assistant = messages
        .iter()
        .take(index)
        .rev()
        .find(|message| counts_for_alternation(message))
        .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("assistant"));
    directly_after_assistant || previous_counted_assistant
}

// Assistant placeholders only belong between adjacent counted user turns.
#[must_use]
pub fn insert_assistant_continuation(
    messages: &[Value],
    custom: Option<&Value>,
    ensure: bool,
) -> Vec<Value> {
    if !ensure || messages.len() <= 1 {
        return messages.to_vec();
    }
    let placeholder = custom
        .cloned()
        .unwrap_or_else(|| json!({"role": "assistant", "content": ASSISTANT_CONTINUE}));
    let mut previous_counted_user = false;
    let mut insertions = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let counted_user = counts_for_alternation(message)
            && message.get("role").and_then(Value::as_str) == Some("user");
        if counted_user && previous_counted_user {
            insertions.push(index);
        }
        if counts_for_alternation(message) {
            previous_counted_user = message.get("role").and_then(Value::as_str) == Some("user");
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
    messages: &[Value],
    assistant: Option<&Value>,
    user: Option<&Value>,
    ensure: bool,
) -> Vec<Value> {
    if !ensure {
        return messages.to_vec();
    }
    let with_users = insert_user_continue_message(messages, user, ensure);
    insert_assistant_continuation(&with_users, assistant, ensure)
}

mod tests;
