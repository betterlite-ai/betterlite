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
    pub(crate) thinking_blocks: Option<&'a Json>,
}

fn message_json(message: &ChatMessage, model: &str) -> Result<Json, CompactionError> {
    let value = serde_json::to_value(message).map_err(|source| CompactionError {
        kind: ErrorKind::Rejected,
        model: model.to_owned(),
        reason: format!("the chat message cannot be read as history: {source}"),
    })?;
    serde_json::from_value(value).map_err(|source| CompactionError {
        kind: ErrorKind::Rejected,
        model: model.to_owned(),
        reason: format!("the chat message cannot be read as history: {source}"),
    })
}

fn item_blocks<'a>(fields: &'a Json) -> Vec<Block<'a>> {
    let mut blocks = Vec::new();
    let Json::Object(fields) = fields else {
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

fn tool_call_ids<'a>(fields: &'a Json) -> Vec<Option<&'a str>> {
    let Json::Object(fields) = fields else {
        return Vec::new();
    };
    let Some(Json::List(calls)) = fields.get("tool_calls") else {
        return Vec::new();
    };
    calls
        .iter()
        .map(|call| match call {
            Json::Object(fields) => field_text(fields, "id"),
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
        role: fields.and_then(|fields| field_text(fields, "role")),
        kind: fields.and_then(|fields| field_text(fields, "type")),
        blocks: item_blocks(item),
        tool_calls: tool_call_ids(item),
        tool_call_id: fields.and_then(|fields| field_text(fields, "tool_call_id")),
        call_id: fields.and_then(|fields| field_text(fields, "call_id")),
        thinking_blocks: fields.and_then(|fields| fields.get("thinking_blocks")),
    }
}

pub(crate) fn tool_ids_of(items: &[Json], results: bool) -> Vec<String> {
    let mut ids = Vec::new();
    for item in items {
        let item = json_item(item);
        if results && item.role == Some("tool") {
            ids.push(item.tool_call_id.unwrap_or_default().to_owned());
        }
        let function_kind = if results {
            "function_call_output"
        } else {
            "function_call"
        };
        if item.kind == Some(function_kind) {
            ids.push(item.call_id.unwrap_or_default().to_owned());
        }
        let block_kind = if results { "tool_result" } else { "tool_use" };
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
