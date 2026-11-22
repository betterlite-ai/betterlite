use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use crate::api::messages_context_clear::constants::COMPACT_SUMMARY_SYSTEM_PREFIX;
use crate::api::summary_history::boundary::block_has_type;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Blocks(Vec<Json>),
}

#[must_use]
pub fn text_part(text: &str) -> Json {
    let mut part = Object::new();
    part.insert("type".to_owned(), Json::Text("text".to_owned()));
    part.insert("text".to_owned(), Json::Text(text.to_owned()));
    Json::Object(part)
}

#[must_use]
pub fn text_turn(role: &str, content: &str) -> Json {
    let mut turn = Object::new();
    turn.insert("role".to_owned(), Json::Text(role.to_owned()));
    turn.insert("content".to_owned(), Json::Text(content.to_owned()));
    Json::Object(turn)
}

#[must_use]
pub fn augment_system_with_summary(system: Option<&SystemPrompt>, summary: &str) -> SystemPrompt {
    let prefix = format!("{COMPACT_SUMMARY_SYSTEM_PREFIX}{summary}\n\n");
    match system {
        None => SystemPrompt::Text(prefix.trim_end().to_owned()),
        Some(SystemPrompt::Text(text)) => SystemPrompt::Text(format!("{prefix}{text}")),
        Some(SystemPrompt::Blocks(blocks)) => {
            let Some(index) = blocks
                .iter()
                .position(|block| block_has_type(block, "text"))
            else {
                let mut prefixed = vec![text_part(prefix.trim_end())];
                prefixed.extend(blocks.iter().cloned());
                return SystemPrompt::Blocks(prefixed);
            };
            let mut augmented = blocks.clone();
            if let Some(Json::Object(fields)) = augmented.get_mut(index) {
                let existing = fields
                    .get("text")
                    .and_then(Json::as_str)
                    .unwrap_or_default();
                let text = format!("{prefix}{existing}");
                fields.insert("text".to_owned(), Json::Text(text));
            }
            SystemPrompt::Blocks(augmented)
        }
    }
}

fn block_texts(blocks: &[Json]) -> Vec<&str> {
    blocks
        .iter()
        .filter(|block| block_has_type(block, "text"))
        .filter_map(|block| block.as_object()?.get("text")?.as_str())
        .filter(|text| !text.is_empty())
        .collect()
}

// token counting joins system blocks with one newline, the summary request with two
#[must_use]
pub fn system_to_text(system: Option<&SystemPrompt>) -> String {
    match system {
        None => String::new(),
        Some(SystemPrompt::Text(text)) => text.clone(),
        Some(SystemPrompt::Blocks(blocks)) => block_texts(blocks).join("\n"),
    }
}

#[must_use]
pub fn system_to_openai_message(system: Option<&SystemPrompt>) -> Option<Json> {
    let content = match system? {
        SystemPrompt::Text(text) => text.clone(),
        SystemPrompt::Blocks(blocks) => block_texts(blocks).join("\n\n"),
    };
    (!content.is_empty()).then(|| text_turn("system", &content))
}
