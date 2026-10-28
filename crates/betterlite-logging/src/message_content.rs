use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

pub fn role_for(value: &str, has_tool_calls: bool) -> ChatRole {
    match value {
        "system" => ChatRole::System,
        "tool" => ChatRole::Tool,
        "assistant" | "ai" => ChatRole::Assistant,
        "user" | "human" => ChatRole::User,
        _ if has_tool_calls => ChatRole::Assistant,
        _ => ChatRole::User,
    }
}

pub fn content_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(blocks) => text_blocks(blocks),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn text_blocks(blocks: &[serde_json::Value]) -> String {
    let mut texts = Vec::new();
    for block in blocks {
        let Some(object) = block.as_object() else {
            return serde_json::to_string(blocks).unwrap_or_default();
        };
        let kind = object
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        match object.get("text") {
            Some(serde_json::Value::String(text)) => texts.push(text.clone()),
            Some(_) => return serde_json::to_string(blocks).unwrap_or_default(),
            None if matches!(
                kind,
                "reasoning"
                    | "thinking"
                    | "redacted_thinking"
                    | "function_call"
                    | "tool_use"
                    | "tool_call"
            ) => {}
            None => return serde_json::to_string(blocks).unwrap_or_default(),
        }
    }
    texts.join("\n\n")
}
