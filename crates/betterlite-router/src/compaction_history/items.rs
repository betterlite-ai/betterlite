use betterlite_models::decisions::Json;

use super::blocks::field_text;

pub(crate) struct Block<'a> {
    pub(crate) kind: Option<&'a str>,
    pub(crate) id: Option<&'a str>,
    pub(crate) tool_use_id: Option<&'a str>,
}

pub(crate) struct Item<'a> {
    pub(crate) role: Option<&'a str>,
    pub(crate) kind: Option<&'a str>,
    pub(crate) blocks: Vec<Block<'a>>,
    pub(crate) tool_calls: Vec<Option<&'a str>>,
    pub(crate) tool_call_id: Option<&'a str>,
    pub(crate) call_id: Option<&'a str>,
}

fn item_blocks(item: &Json) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let Json::Object(fields) = item else {
        return blocks;
    };
    let Some(Json::List(entries)) = fields.get("content") else {
        return blocks;
    };
    for entry in entries {
        match entry {
            Json::Object(block) => blocks.push(Block {
                kind: field_text(block, "type"),
                id: field_text(block, "id"),
                tool_use_id: field_text(block, "tool_use_id"),
            }),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                blocks.push(Block {
                    kind: None,
                    id: None,
                    tool_use_id: None,
                });
            }
        }
    }
    blocks
}

fn tool_call_ids(item: &Json) -> Vec<Option<&str>> {
    let Json::Object(fields) = item else {
        return Vec::new();
    };
    let Some(Json::List(calls)) = fields.get("tool_calls") else {
        return Vec::new();
    };
    calls
        .iter()
        .map(|call| match call {
            Json::Object(call_fields) => field_text(call_fields, "id"),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

pub(crate) fn json_item(item: &Json) -> Item<'_> {
    let fields = match item {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    Item {
        role: fields.and_then(|object| field_text(object, "role")),
        kind: fields.and_then(|object| field_text(object, "type")),
        blocks: item_blocks(item),
        tool_calls: tool_call_ids(item),
        tool_call_id: fields.and_then(|object| field_text(object, "tool_call_id")),
        call_id: fields.and_then(|object| field_text(object, "call_id")),
    }
}

pub(crate) fn tool_ids_of(items: &[Json], results: bool) -> Vec<String> {
    let function_kind = if results {
        "function_call_output"
    } else {
        "function_call"
    };
    let block_kind = if results { "tool_result" } else { "tool_use" };
    let mut ids = Vec::new();
    for raw in items {
        let item = json_item(raw);
        if results && item.role == Some("tool") {
            ids.push(item.tool_call_id.unwrap_or_default().to_owned());
        }
        if item.kind == Some(function_kind) {
            ids.push(item.call_id.unwrap_or_default().to_owned());
        }
        for block in &item.blocks {
            if block.kind == Some(block_kind) {
                let id = if results { block.tool_use_id } else { block.id };
                ids.push(id.unwrap_or_default().to_owned());
            }
        }
        if !results {
            for call in &item.tool_calls {
                ids.push(call.unwrap_or_default().to_owned());
            }
        }
    }
    ids
}
