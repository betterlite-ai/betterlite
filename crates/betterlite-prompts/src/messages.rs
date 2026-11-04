use serde_json::{Value, json};

const USER_CONTINUE: &str = "Please continue.";
const ASSISTANT_CONTINUE: &str = "Continue.";

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
    if result
        .first()
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        == Some("assistant")
    {
        result.insert(0, placeholder.clone());
    }
    let mut index = 1;
    while index < result.len() {
        let current_assistant = result.get(index).is_some_and(|m| {
            counts_for_alternation(m) && m.get("role").and_then(Value::as_str) == Some("assistant")
        });
        if current_assistant {
            let previous_is_assistant = result
                .get(index - 1)
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                == Some("assistant");
            let previous_counted_assistant = result[..index]
                .iter()
                .rev()
                .find(|m| counts_for_alternation(m))
                .is_some_and(|m| m.get("role").and_then(Value::as_str) == Some("assistant"));
            if previous_is_assistant || previous_counted_assistant {
                result.insert(index, placeholder.clone());
                index += 1;
            }
        }
        index += 1;
    }
    if ensure
        && result
            .last()
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            == Some("assistant")
    {
        result.push(placeholder);
    }
    result
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
    let mut result = Vec::with_capacity(messages.len());
    let mut previous_counted_user = false;
    for message in messages {
        let counted_user = counts_for_alternation(message)
            && message.get("role").and_then(Value::as_str) == Some("user");
        if counted_user && previous_counted_user {
            result.push(placeholder.clone());
        }
        result.push(message.clone());
        if counts_for_alternation(message) {
            previous_counted_user = message.get("role").and_then(Value::as_str) == Some("user");
        }
    }
    result
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
