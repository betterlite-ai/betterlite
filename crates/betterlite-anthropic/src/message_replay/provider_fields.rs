use betterlite_prompts::arguments::json::Json;

#[must_use]
pub fn strip_provider_specific_fields(messages: &[Json]) -> Vec<Json> {
    let mut stripped = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(fields) = message.as_object() else {
            stripped.push(message.clone());
            continue;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            stripped.push(message.clone());
            continue;
        };
        let content: Vec<Json> = blocks.iter().map(strip_provider_block).collect();
        let mut copy = fields.clone();
        copy.insert("content".to_owned(), Json::List(content));
        stripped.push(Json::Object(copy));
    }
    stripped
}

fn strip_provider_block(block: &Json) -> Json {
    let Some(fields) = block.as_object() else {
        return block.clone();
    };
    if !fields.contains_key("provider_specific_fields") {
        return block.clone();
    }
    let mut copy = fields.clone();
    copy.shift_remove("provider_specific_fields");
    Json::Object(copy)
}
