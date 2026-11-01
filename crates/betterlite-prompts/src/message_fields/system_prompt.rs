use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, ContentPart, Message};

#[must_use]
pub fn add_system_prompt_to_messages(
    messages: &[Message],
    system_prompt: &str,
    merge_with_first_system: bool,
) -> Vec<Message> {
    if system_prompt.is_empty() {
        return messages.to_vec();
    }
    if merge_with_first_system
        && let Some((first, rest)) = messages.split_first()
        && first.role == "system"
    {
        let mut merged = Vec::with_capacity(messages.len());
        merged.push(merge_system_prompt(first, system_prompt));
        merged.extend_from_slice(rest);
        return merged;
    }
    let mut with_prompt = Vec::with_capacity(messages.len().saturating_add(1));
    with_prompt.push(Message {
        role: "system".to_owned(),
        content: Some(Content::Text(system_prompt.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    });
    with_prompt.extend_from_slice(messages);
    with_prompt
}

fn merge_system_prompt(first: &Message, system_prompt: &str) -> Message {
    let trimmed_prompt = system_prompt.trim();
    let mut merged = first.clone();
    merged.content = Some(match &first.content {
        Some(Content::Text(existing)) => Content::Text(format!("{trimmed_prompt}\n\n{existing}")),
        Some(Content::Parts(parts)) => {
            let prompt_part = ContentPart {
                kind: "text".to_owned(),
                fields: Object::from_iter([(
                    "text".to_owned(),
                    Json::Text(trimmed_prompt.to_owned()),
                )]),
            };
            let mut merged_parts = vec![prompt_part];
            merged_parts.extend(parts.iter().cloned());
            Content::Parts(merged_parts)
        }
        None => Content::Text(format!("{trimmed_prompt}\n\n")),
    });
    merged
}

mod tests;
