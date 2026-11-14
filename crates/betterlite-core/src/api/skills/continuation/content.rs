use serde_json::{Map, Value};

use crate::api::skills::hook::response::{ContentBlock, HookResponse};

#[must_use]
pub fn stop_reason(response: &HookResponse) -> Option<&str> {
    match response {
        HookResponse::Dict(fields) => fields.get("stop_reason").and_then(Value::as_str),
        HookResponse::Object(object) => object.stop_reason.as_deref(),
    }
}

#[must_use]
pub fn assistant_message(response: &HookResponse) -> Value {
    let blocks: Vec<Value> = match response {
        HookResponse::Dict(fields) => match fields.get("content") {
            Some(Value::Array(blocks)) => blocks.iter().map(dict_block).collect(),
            _ => Vec::new(),
        },
        HookResponse::Object(object) => object.content.iter().map(object_block).collect(),
    };
    message("assistant", blocks)
}

#[must_use]
pub fn message(role: &str, content: Vec<Value>) -> Value {
    let mut fields = Map::new();
    fields.insert("role".to_owned(), Value::String(role.to_owned()));
    fields.insert("content".to_owned(), Value::Array(content));
    Value::Object(fields)
}

#[must_use]
pub fn tool_result(tool_use_id: Option<&str>, content: String) -> Value {
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String("tool_result".to_owned()));
    fields.insert(
        "tool_use_id".to_owned(),
        tool_use_id.map_or(Value::Null, |id| Value::String(id.to_owned())),
    );
    fields.insert("content".to_owned(), Value::String(content));
    Value::Object(fields)
}

// LiteLLM stringifies a non-object entry with Python's str(), which docs/compatibility.md excludes
fn dict_block(block: &Value) -> Value {
    if block.is_object() {
        return block.clone();
    }
    let mut text_block = Map::new();
    text_block.insert("type".to_owned(), Value::String("text".to_owned()));
    text_block.insert("text".to_owned(), Value::String(block.to_string()));
    Value::Object(text_block)
}

fn object_block(block: &ContentBlock) -> Value {
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String(block.kind.clone()));
    if let Some(id) = &block.id {
        fields.insert("id".to_owned(), Value::String(id.clone()));
    }
    if let Some(name) = &block.name {
        fields.insert("name".to_owned(), Value::String(name.clone()));
    }
    fields.insert("input".to_owned(), block.input.clone());
    if let Some(text) = &block.text {
        fields.insert("text".to_owned(), Value::String(text.clone()));
    }
    Value::Object(fields)
}
