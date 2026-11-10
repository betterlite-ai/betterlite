use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::messages_context_clear::constants::CLEARED_TOOL_RESULT_CONTENT;

fn content_blocks(message: &Json) -> Option<&[Json]> {
    let Json::Object(fields) = message else {
        return None;
    };
    let Some(Json::List(blocks)) = fields.get("content") else {
        return None;
    };
    Some(blocks)
}

const fn block_object(block: &Json) -> Option<&IndexMap<String, Json>> {
    match block {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn block_kind(block: &Json) -> Option<&str> {
    match block_object(block)?.get("type")? {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn block_text_field<'block>(block: &'block Json, key: &str) -> Option<&'block str> {
    match block_object(block)?.get(key)? {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub struct ToolIds<'message> {
    pub uses: Vec<&'message str>,
    pub last_result: Option<&'message str>,
}

#[must_use]
pub fn tool_ids(messages: &[Json]) -> ToolIds<'_> {
    let mut uses = Vec::new();
    let mut last_result = None;
    for message in messages {
        let Some(blocks) = content_blocks(message) else {
            continue;
        };
        for block in blocks {
            if block_kind(block) == Some("tool_use")
                && let Some(id) = block_text_field(block, "id")
            {
                uses.push(id);
            }
            if block_kind(block) == Some("tool_result")
                && let Some(id) = block_text_field(block, "tool_use_id")
            {
                last_result = Some(id);
            }
        }
    }
    ToolIds { uses, last_result }
}

#[must_use]
pub fn cleared_tool_result_content(original: Option<&Json>) -> Json {
    match original {
        Some(Json::List(_)) => {
            let mut block = IndexMap::new();
            block.insert("type".to_owned(), Json::Text("text".to_owned()));
            block.insert(
                "text".to_owned(),
                Json::Text(CLEARED_TOOL_RESULT_CONTENT.to_owned()),
            );
            Json::List(vec![Json::Object(block)])
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Json::Text(CLEARED_TOOL_RESULT_CONTENT.to_owned()),
    }
}

fn cleared_result_block(block: &Json, ids_to_clear: &BTreeSet<&str>) -> Option<Json> {
    if block_kind(block) != Some("tool_result") {
        return None;
    }
    let id = block_text_field(block, "tool_use_id")?;
    if !ids_to_clear.contains(id) {
        return None;
    }
    let fields = block_object(block)?;
    let mut copy = fields.clone();
    copy.insert(
        "content".to_owned(),
        cleared_tool_result_content(fields.get("content")),
    );
    Some(Json::Object(copy))
}

#[must_use]
pub fn clear_tool_results(messages: &[Json], ids_to_clear: &BTreeSet<&str>) -> (Vec<Json>, usize) {
    let mut cleared = 0_usize;
    let mut edited = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(blocks) = content_blocks(message) else {
            edited.push(message.clone());
            continue;
        };
        let mut new_blocks = Vec::with_capacity(blocks.len());
        let mut mutated = false;
        for block in blocks {
            match cleared_result_block(block, ids_to_clear) {
                Some(result) => {
                    new_blocks.push(result);
                    cleared = cleared.saturating_add(1);
                    mutated = true;
                }
                None => new_blocks.push(block.clone()),
            }
        }
        match message {
            Json::Object(fields) if mutated => {
                let mut copy = fields.clone();
                copy.insert("content".to_owned(), Json::List(new_blocks));
                edited.push(Json::Object(copy));
            }
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => edited.push(message.clone()),
        }
    }
    (edited, cleared)
}
