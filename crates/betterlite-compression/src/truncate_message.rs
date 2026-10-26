use crate::extract_key::content_to_string;
use serde_json::Value;

pub fn truncate_message(message: &Value, max_tokens: usize) -> Value {
    let content = message
        .get("content")
        .map(content_to_string)
        .unwrap_or_default();
    let target_chars = max_tokens.saturating_mul(3).max(100);
    if content.len() <= target_chars {
        return replace_content(message, content);
    }
    let lines: Vec<&str> = content.split('\n').collect();
    let average = (content.len() / lines.len()).max(1);
    let target_lines = (target_chars / average).max(2);
    if lines.len() <= target_lines {
        return replace_content(message, content);
    }
    let first_count = target_lines * 7 / 10;
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

#[cfg(test)]
mod tests;
