use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};

// _hidden_params["mcp_metadata"], which only a non-empty object counts as.
pub(super) fn metadata(fields: Option<&JsonObject>) -> Option<&JsonObject> {
    fields.filter(|fields| !fields.is_empty())
}

// _add_mcp_list_tools_to_first_chunk: only the tool list, never tool results or the other keys.
pub(super) fn add_list_tools(chunk: &mut StreamChunk, metadata: Option<&JsonObject>) {
    let Some(fields) = metadata else {
        return;
    };
    let Some(tools) = fields
        .get("mcp_list_tools")
        .filter(|tools| holds_metadata(tools))
    else {
        return;
    };
    for choice in &mut chunk.choices {
        choice
            .delta
            .provider_specific_fields
            .get_or_insert_with(JsonObject::new)
            .insert("mcp_list_tools".to_owned(), tools.clone());
    }
}

// _add_mcp_metadata_to_final_chunk: every key of mcp_metadata, replacing what the delta carried.
pub(super) fn add_final_metadata(chunk: &mut StreamChunk, metadata: Option<&JsonObject>) {
    let Some(fields) = metadata else {
        return;
    };
    for choice in &mut chunk.choices {
        choice
            .delta
            .provider_specific_fields
            .get_or_insert_with(JsonObject::new)
            .extend(fields.clone());
    }
}

// A metadata value is added only when Python's `not mcp_list_tools` would let it through.
fn holds_metadata(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|amount| amount.abs() > 0.0),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
