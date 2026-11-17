use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, item_count,
};

// _add_mcp_list_tools_to_first_chunk: only the tool list, never tool results or the other keys.
pub(super) fn add_list_tools(chunk: &mut StreamChunk, metadata: Option<&JsonObject>) {
    let Some(fields) = metadata.filter(|fields| !fields.is_empty()) else {
        return;
    };
    let Some(tools) = fields
        .get("mcp_list_tools")
        .filter(|tools| holds_metadata(tools))
    else {
        return;
    };
    let mut selected = JsonObject::new();
    selected.insert("mcp_list_tools".to_owned(), tools.clone());
    add_to_choices(chunk, &selected);
}

// _add_mcp_metadata_to_final_chunk: every key of mcp_metadata, replacing what the delta carried.
pub(super) fn add_final_metadata(chunk: &mut StreamChunk, metadata: Option<&JsonObject>) {
    let Some(fields) = metadata.filter(|fields| !fields.is_empty()) else {
        return;
    };
    add_to_choices(chunk, fields);
}

fn add_to_choices(chunk: &mut StreamChunk, fields: &JsonObject) {
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
        JsonValue::Number(number) => number.as_f64().is_some_and(|amount| amount != 0.0_f64),
        JsonValue::String(_) | JsonValue::Array(_) | JsonValue::Object(_) => item_count(value) > 0,
    }
}
