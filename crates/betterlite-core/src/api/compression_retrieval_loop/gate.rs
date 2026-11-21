use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::compression_interception::hook::CompressionInterception;
use crate::api::compression_interception::tools::{CONTENT_RETRIEVE_TOOL_NAME, has_retrieval_tool};

use super::{
    COMPRESSION_RETRIEVAL_TYPE, CONTENT_FIELD, ID_FIELD, INPUT_FIELD, NAME_FIELD,
    THINKING_BLOCKS_FIELD, TOOL_CALLS_FIELD, TOOL_TYPE_FIELD, TOOL_USE_TYPE, TYPE_FIELD,
};

pub(super) fn should_run_agentic_loop<Engine>(
    interception: &CompressionInterception<Engine>,
    response: &Json,
    tools: Option<&Json>,
) -> (bool, IndexMap<String, Json>) {
    if !interception.settings.enabled || !has_retrieval_tool(tools) {
        return (false, IndexMap::new());
    }
    let (tool_calls, thinking_blocks) = extract_retrieval_blocks(response);
    if tool_calls.is_empty() {
        return (false, IndexMap::new());
    }
    let mut ready = IndexMap::with_capacity(3);
    ready.insert(TOOL_CALLS_FIELD.to_owned(), Json::List(tool_calls));
    ready.insert(
        THINKING_BLOCKS_FIELD.to_owned(),
        Json::List(thinking_blocks),
    );
    ready.insert(
        TOOL_TYPE_FIELD.to_owned(),
        Json::Text(COMPRESSION_RETRIEVAL_TYPE.to_owned()),
    );
    (true, ready)
}

pub(super) fn extract_retrieval_blocks(response: &Json) -> (Vec<Json>, Vec<Json>) {
    let Json::Object(fields) = response else {
        return (Vec::new(), Vec::new());
    };
    let Some(Json::List(content)) = fields.get(CONTENT_FIELD) else {
        return (Vec::new(), Vec::new());
    };
    let mut tool_calls = Vec::new();
    let mut thinking_blocks = Vec::new();
    for block in content {
        let Json::Object(block_fields) = block else {
            continue;
        };
        let Some(Json::Text(block_type)) = block_fields.get(TYPE_FIELD) else {
            continue;
        };
        if matches!(block_type.as_str(), "thinking" | "redacted_thinking") {
            thinking_blocks.push(block.clone());
        }
        let holds_retrieval_name = matches!(
            block_fields.get(NAME_FIELD),
            Some(Json::Text(name)) if name.as_str() == CONTENT_RETRIEVE_TOOL_NAME
        );
        if block_type.as_str() == TOOL_USE_TYPE && holds_retrieval_name {
            tool_calls.push(retrieval_call(block_fields));
        }
    }
    (tool_calls, thinking_blocks)
}

fn retrieval_call(block: &IndexMap<String, Json>) -> Json {
    let mut call = IndexMap::with_capacity(4);
    call.insert(
        ID_FIELD.to_owned(),
        block.get(ID_FIELD).cloned().unwrap_or(Json::Null),
    );
    call.insert(TYPE_FIELD.to_owned(), Json::Text(TOOL_USE_TYPE.to_owned()));
    call.insert(
        NAME_FIELD.to_owned(),
        Json::Text(CONTENT_RETRIEVE_TOOL_NAME.to_owned()),
    );
    call.insert(INPUT_FIELD.to_owned(), block_input(block));
    Json::Object(call)
}

pub(super) fn block_input(block: &IndexMap<String, Json>) -> Json {
    match block.get(INPUT_FIELD) {
        None | Some(Json::Null) => Json::Object(IndexMap::new()),
        Some(value) => value.clone(),
    }
}
