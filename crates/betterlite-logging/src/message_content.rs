use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::python_json::dumps;

mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

impl ChatRole {
    // LiteLLM's MESSAGE_ROLES: the OpenAI roles plus LangChain's human and ai message types
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "human" | "user" => Some(Self::User),
            "ai" | "assistant" => Some(Self::Assistant),
            "system" => Some(Self::System),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

const NON_TEXT_BLOCKS: [&str; 6] = [
    "reasoning",
    "thinking",
    "redacted_thinking",
    "function_call",
    "tool_use",
    "tool_call",
];

#[must_use]
pub fn content_text(content: &Json) -> String {
    match content {
        Json::Null => String::new(),
        Json::Text(text) => text.clone(),
        Json::List(blocks) => {
            block_texts(blocks).map_or_else(|| dumps(content), |texts| texts.join("\n\n"))
        }
        Json::Bool(_) | Json::Number(_) | Json::Object(_) => dumps(content),
    }
}

// A block is an object whose type is a string and text a string or null; any other block shows the raw JSON
fn block_texts(blocks: &[Json]) -> Option<Vec<&str>> {
    let mut texts = Vec::new();
    for block in blocks {
        let Json::Object(fields) = block else {
            return None;
        };
        let kind = match fields.get("type") {
            None => "",
            Some(Json::Text(kind)) => kind.as_str(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => return None,
        };
        match fields.get("text") {
            Some(Json::Text(text)) => texts.push(text.as_str()),
            None | Some(Json::Null) if NON_TEXT_BLOCKS.contains(&kind) => {}
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => {
                return None;
            }
        }
    }
    Some(texts)
}
