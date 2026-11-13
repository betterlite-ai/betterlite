use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputText};
use indexmap::IndexMap;

use crate::api::responses_tool_namespaces::values::has_value;

use super::error::ChatOutputError;
use super::output::OutputItem;
use super::records::ChatChoice;
use super::status::finish_status;

pub fn reasoning_items<F>(
    choices: &[ChatChoice],
    uuid: &mut F,
) -> Result<Vec<OutputItem>, ChatOutputError>
where
    F: FnMut() -> String,
{
    for choice in choices {
        let message = &choice.message;
        let reasoning = message
            .reasoning_content
            .as_deref()
            .filter(|text| !text.is_empty());
        let encrypted = encrypted_content(&message.thinking_blocks)?;
        if reasoning.is_none() && encrypted.is_none() {
            continue;
        }
        let content = reasoning.map_or_else(Vec::new, |text| {
            vec![OutputText {
                r#type: Some("output_text".to_owned()),
                text: Some(text.to_owned()),
                annotations: Some(Vec::new()),
                extra: BTreeMap::new(),
            }]
        });
        let mut extra = BTreeMap::new();
        extra.insert(
            "encrypted_content".to_owned(),
            encrypted.map_or(Json::Null, Json::Text),
        );
        return Ok(vec![OutputItem::Reasoning(GenericResponseOutputItem {
            r#type: "reasoning".to_owned(),
            id: format!("rs_{}", uuid()),
            status: finish_status(choice.finish_reason.as_deref()).to_owned(),
            role: "assistant".to_owned(),
            content,
            phase: None,
            extra,
        })]);
    }
    Ok(Vec::new())
}

fn encrypted_content(blocks: &[IndexMap<String, Json>]) -> Result<Option<String>, ChatOutputError> {
    let mut preserved = Vec::new();
    for block in blocks {
        let kept = block.get("signature").is_some_and(has_value)
            || block.get("data").is_some_and(has_value);
        if kept {
            preserved.push(normalized_block(block)?);
        }
    }
    if preserved.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&preserved)
        .map(Some)
        .map_err(|source| ChatOutputError::Json {
            field: "thinking_blocks",
            source,
        })
}

// pydantic writes a validated TypedDict's declared keys first, in declaration order, then the keys it kept as extras
fn normalized_block(
    block: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, ChatOutputError> {
    let known: &[&str] = match block.get("type") {
        Some(Json::Text(kind)) if kind == "thinking" => {
            &["type", "thinking", "signature", "cache_control"]
        }
        Some(Json::Text(kind)) if kind == "redacted_thinking" => &["type", "data", "cache_control"],
        _ => {
            return Err(ChatOutputError::WrongType {
                field: "thinking_blocks",
                expected: "a thinking block with a known type",
            });
        }
    };
    let mut normalized = IndexMap::new();
    for name in known {
        if let Some(value) = block.get(*name) {
            normalized.insert((*name).to_owned(), value.clone());
        }
    }
    for (name, value) in block {
        if !known.contains(&name.as_str()) {
            normalized.insert(name.clone(), value.clone());
        }
    }
    Ok(normalized)
}
