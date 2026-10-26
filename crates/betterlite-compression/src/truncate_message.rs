use crate::extract_key::content_to_string;
use serde_json::Value;

pub fn truncate_message(message: &Value, max_tokens: usize) -> Value {
    let content = message
        .get("content")
        .map(content_to_string)
        .unwrap_or_default();
    let target_chars = max_tokens.saturating_mul(3).max(100);
    let characters = content.chars().count();
    if characters <= target_chars {
        return replace_content(message, content);
    }
    let lines: Vec<&str> = content.split('\n').collect();
    let average = characters.checked_div(lines.len()).unwrap_or(0).max(1);
    let target_lines = target_chars.checked_div(average).unwrap_or(0).max(2);
    if lines.len() <= target_lines {
        return replace_content(message, content);
    }
    let first_count = target_lines
        .checked_mul(7)
        .and_then(|count| count.checked_div(10))
        .unwrap_or(0);
    let last_count = target_lines - first_count;
    let first = lines
        .iter()
        .take(first_count)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let last = lines
        .iter()
        .rev()
        .take(last_count)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    replace_content(
        message,
        format!("{first}\n...[truncated for context window]...\n{last}"),
    )
}

fn replace_content(message: &Value, content: String) -> Value {
    let mut result = message.as_object().cloned().unwrap_or_default();
    result.insert("content".to_owned(), Value::String(content));
    Value::Object(result)
}

mod tests;
