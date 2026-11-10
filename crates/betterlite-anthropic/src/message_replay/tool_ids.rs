use betterlite_prompts::arguments::json::{Json, Object};

const THOUGHT_SIGNATURE_SEPARATOR: &str = "__thought__";
const TOOL_USE_ID_FALLBACK: &str = "tool_use_id";

#[must_use]
pub fn sanitize_tool_use_ids(messages: &[Json]) -> Vec<Json> {
    let mut sanitized = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(fields) = message.as_object() else {
            sanitized.push(message.clone());
            continue;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            sanitized.push(message.clone());
            continue;
        };
        let replaced: Vec<Json> = blocks.iter().map(sanitize_block).collect();
        if replaced == *blocks {
            sanitized.push(message.clone());
        } else {
            let mut copy = fields.clone();
            copy.insert("content".to_owned(), Json::List(replaced));
            sanitized.push(Json::Object(copy));
        }
    }
    sanitized
}

fn sanitize_block(block: &Json) -> Json {
    let Some(fields) = block.as_object() else {
        return block.clone();
    };
    match fields.get("type").and_then(Json::as_str) {
        Some("tool_use" | "server_tool_use") => sanitized_id(block, fields, "id"),
        Some("tool_result") => sanitized_id(block, fields, "tool_use_id"),
        Some(_) | None => block.clone(),
    }
}

fn sanitized_id(block: &Json, fields: &Object, key: &str) -> Json {
    let Some(raw_id) = fields.get(key).and_then(Json::as_str) else {
        return block.clone();
    };
    let normalized = normalize_tool_use_id(raw_id);
    if normalized == raw_id {
        return block.clone();
    }
    let mut copy = fields.clone();
    copy.insert(key.to_owned(), Json::Text(normalized));
    Json::Object(copy)
}

#[must_use]
pub fn normalize_tool_use_id(raw_id: &str) -> String {
    let base_id = raw_id
        .split_once(THOUGHT_SIGNATURE_SEPARATOR)
        .map_or(raw_id, |(base_id, _)| base_id);
    let sanitized: String = base_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        TOOL_USE_ID_FALLBACK.to_owned()
    } else {
        sanitized
    }
}
