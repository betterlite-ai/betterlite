use betterlite_prompts::arguments::json::{Json, Object};

// Azure AI Foundry accepts only type and ttl on cache_control, so a block's scope goes
pub fn transform_anthropic_messages_request(request: &mut Object) {
    if let Some(Json::List(system)) = request.get_mut("system") {
        remove_scope(system);
    }
    if let Some(Json::List(messages)) = request.get_mut("messages") {
        for message in messages.iter_mut().filter_map(object_fields) {
            if let Some(Json::List(content)) = message.get_mut("content") {
                remove_scope(content);
            }
        }
    }
}

const fn object_fields(value: &mut Json) -> Option<&mut Object> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn remove_scope(blocks: &mut [Json]) {
    for block in blocks.iter_mut().filter_map(object_fields) {
        if let Some(Json::Object(cache_control)) = block.get_mut("cache_control") {
            cache_control.shift_remove("scope");
        }
    }
}
