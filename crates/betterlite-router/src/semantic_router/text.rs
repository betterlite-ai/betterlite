use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};

// LiteLLM reads the last user message: a null content routes on nothing, a parts list on its text blocks
#[must_use]
pub fn last_user_text(messages: &[ChatMessage]) -> String {
    for message in messages.iter().rev() {
        if message.role != Role::User {
            continue;
        }
        return match &message.content {
            None => String::new(),
            Some(MessageContent::Text(text)) => text.clone(),
            Some(MessageContent::Parts(parts)) => text_parts(parts),
        };
    }
    String::new()
}

fn text_parts(parts: &[ContentPart]) -> String {
    let mut texts: Vec<&str> = Vec::new();
    for part in parts {
        if part.kind != "text" {
            continue;
        }
        let Some(value) = part.fields.get("text") else {
            continue;
        };
        let Some(text) = value.as_str() else {
            continue;
        };
        texts.push(text);
    }
    texts.join(" ")
}

// LiteLLM cuts by characters, which are Unicode scalar values, and keeps the head
#[must_use]
pub fn clamp(text: &str, max_input_chars: i64) -> &str {
    if max_input_chars <= 0 {
        return text;
    }
    let limit = usize::try_from(max_input_chars).unwrap_or(usize::MAX);
    match text.char_indices().nth(limit) {
        Some((end, _)) => text.split_at(end).0,
        None => text,
    }
}
