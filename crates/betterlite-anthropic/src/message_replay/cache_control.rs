use betterlite_prompts::arguments::json::{Json, Object};

// LiteLLM copies the payload and reduces each cache hint to what the Messages API defines.
#[must_use]
pub fn normalize_cache_control(payload: &Object) -> Object {
    let portable = portable_block(payload);
    portable
        .into_iter()
        .map(|(key, value)| {
            let scoped = portable_scoped_value(&key, &value);
            (key, scoped)
        })
        .collect()
}

fn portable_scoped_value(key: &str, value: &Json) -> Json {
    match key {
        "system" | "tools" => portable_blocks(value),
        "messages" => portable_messages(value),
        _ => value.clone(),
    }
}

// LiteLLM rebuilds the block without cache_control and appends the portable hint last.
fn portable_block(fields: &Object) -> Object {
    let Some(cache_control) = fields.get("cache_control") else {
        return fields.clone();
    };
    let mut rest = fields.clone();
    rest.shift_remove("cache_control");
    if let Some(normalized) = normalized_cache_control(cache_control) {
        rest.insert("cache_control".to_owned(), normalized);
    }
    rest
}

fn normalized_cache_control(cache_control: &Json) -> Option<Json> {
    let fields = cache_control.as_object()?;
    let kind = match fields.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "ephemeral".to_owned(),
    };
    Some(Json::Object(Object::from_iter([(
        "type".to_owned(),
        Json::Text(kind),
    )])))
}

fn portable_blocks(blocks: &Json) -> Json {
    match blocks {
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| match item {
                    Json::Object(fields) => Json::Object(portable_block(fields)),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_) => item.clone(),
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            blocks.clone()
        }
    }
}

fn portable_messages(messages: &Json) -> Json {
    let Json::List(items) = messages else {
        return messages.clone();
    };
    let portable: Vec<Json> = items.iter().map(portable_message).collect();
    Json::List(portable)
}

fn portable_message(message: &Json) -> Json {
    let Json::Object(fields) = message else {
        return message.clone();
    };
    let Some(content) = fields.get("content") else {
        return message.clone();
    };
    let Json::List(blocks) = content else {
        return message.clone();
    };
    let mut copy = fields.clone();
    let portable = blocks.iter().map(portable_content_block).collect();
    copy.insert("content".to_owned(), Json::List(portable));
    Json::Object(copy)
}

fn portable_content_block(block: &Json) -> Json {
    let Json::Object(fields) = block else {
        return block.clone();
    };
    let portable = portable_block(fields);
    if portable.get("type").and_then(Json::as_str) != Some("tool_result") {
        return Json::Object(portable);
    }
    let Some(content) = portable.get("content") else {
        return Json::Object(portable);
    };
    let nested = portable_blocks(content);
    let mut copy = portable;
    copy.insert("content".to_owned(), nested);
    Json::Object(copy)
}
