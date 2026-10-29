#![expect(
    clippy::disallowed_types,
    reason = "Responses blocks are heterogeneous provider JSON forwarded without a closed chat-message schema yet"
)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("Invalid content type: {type_name}")]
    InvalidContentType { type_name: &'static str },
}

const VALID_CHAT_COMPLETION_CONTENT_TYPES: [&str; 12] = [
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
    "thinking",
    "redacted_thinking",
];

fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
    }
}

fn truthy_str(item: &Map<String, Value>, key: &str) -> Option<Value> {
    let value = item.get(key)?;
    is_truthy(Some(value)).then(|| value.clone())
}

#[must_use]
pub fn resolve_file_id(item: &Map<String, Value>) -> Option<Value> {
    truthy_str(item, "file_id").or_else(|| truthy_str(item, "file_url"))
}

#[must_use]
pub fn transform_input_file_item_to_file_item(item: &Map<String, Value>) -> Value {
    let mut file_dict = Map::new();
    if let Some(file_id) = resolve_file_id(item) {
        file_dict.insert("file_id".to_owned(), file_id);
    }
    if let Some(file_data) = truthy_str(item, "file_data") {
        file_dict.insert("file_data".to_owned(), file_data);
    }
    if let Some(filename) = truthy_str(item, "filename") {
        file_dict.insert("filename".to_owned(), filename);
    }

    let mut new_item = Map::new();
    new_item.insert("type".to_owned(), Value::String("file".to_owned()));
    new_item.insert("file".to_owned(), Value::Object(file_dict));
    if let Some(cache_control) = item.get("cache_control") {
        new_item.insert("cache_control".to_owned(), cache_control.clone());
    }
    Value::Object(new_item)
}

#[must_use]
pub fn transform_input_image_item_to_image_item(item: &Map<String, Value>) -> Value {
    let url = truthy_str(item, "image_url").unwrap_or_else(|| Value::String(String::new()));
    let detail = truthy_str(item, "detail").unwrap_or_else(|| Value::String("auto".to_owned()));

    let mut image_url = Map::new();
    image_url.insert("url".to_owned(), url);
    image_url.insert("detail".to_owned(), detail);

    let mut image = Map::new();
    image.insert("type".to_owned(), Value::String("image_url".to_owned()));
    image.insert("image_url".to_owned(), Value::Object(image_url));
    Value::Object(image)
}

#[must_use]
pub fn chat_content_type(content_type: &str) -> &'static str {
    if let Some(stripped) = content_type.strip_prefix("input_") {
        if VALID_CHAT_COMPLETION_CONTENT_TYPES.contains(&stripped) {
            return valid_content_type(stripped);
        }
        if stripped == "audio" {
            return "input_audio";
        }
        return "text";
    }

    if content_type == "tool_result" || content_type == "output_text" {
        return "text";
    }

    if VALID_CHAT_COMPLETION_CONTENT_TYPES.contains(&content_type) {
        return valid_content_type(content_type);
    }

    "text"
}

fn valid_content_type(content_type: &str) -> &'static str {
    VALID_CHAT_COMPLETION_CONTENT_TYPES
        .iter()
        .find(|valid| **valid == content_type)
        .copied()
        .unwrap_or("text")
}

fn python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(flag) => {
            if *flag {
                "True".to_owned()
            } else {
                "False".to_owned()
            }
        }
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn content_list_item(item: &Value) -> Option<Value> {
    match item {
        Value::String(text) => Some(Value::String(text.clone())),
        Value::Object(fields) => content_item_block(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    }
}

fn image_content_block(item: &Map<String, Value>) -> Value {
    let mut image_block = transform_input_image_item_to_image_item(item);
    if let Value::Object(fields) = &mut image_block
        && let Some(cache_control) = item.get("cache_control")
    {
        fields.insert("cache_control".to_owned(), cache_control.clone());
    }
    image_block
}

fn encrypted_content_block(item: &Map<String, Value>) -> Option<Value> {
    let encrypted_content = item.get("encrypted_content")?;
    if encrypted_content.is_null() {
        return None;
    }
    let mut block = Map::new();
    block.insert("type".to_owned(), Value::String("text".to_owned()));
    block.insert(
        "text".to_owned(),
        Value::String(python_str(encrypted_content)),
    );
    Some(Value::Object(block))
}

fn generic_text_block(item: &Map<String, Value>, type_name: Option<&str>) -> Option<Value> {
    let text_value = item.get("text")?;
    if text_value.is_null() {
        return None;
    }
    let content_type = if is_truthy(item.get("type")) {
        type_name.unwrap_or("text")
    } else {
        "text"
    };
    let mut block = Map::new();
    block.insert(
        "type".to_owned(),
        Value::String(chat_content_type(content_type).to_owned()),
    );
    block.insert("text".to_owned(), text_value.clone());
    if let Some(cache_control) = item.get("cache_control") {
        block.insert("cache_control".to_owned(), cache_control.clone());
    }
    Some(Value::Object(block))
}

fn content_item_block(item: &Map<String, Value>) -> Option<Value> {
    let type_name = item.get("type").and_then(Value::as_str);
    match type_name {
        Some("input_file") => Some(transform_input_file_item_to_file_item(item)),
        Some("input_image") => Some(image_content_block(item)),
        Some("encrypted_content") => encrypted_content_block(item),
        _ => generic_text_block(item, type_name),
    }
}

pub fn content_blocks(content: &Value) -> Result<Value, ContentError> {
    match content {
        Value::Null => Ok(Value::String(String::new())),
        Value::String(text) => Ok(Value::String(text.clone())),
        Value::Array(items) => Ok(Value::Array(
            items.iter().filter_map(content_list_item).collect(),
        )),
        Value::Bool(_) => Err(ContentError::InvalidContentType {
            type_name: "<class 'bool'>",
        }),
        Value::Number(number) => Err(ContentError::InvalidContentType {
            type_name: if number.is_f64() {
                "<class 'float'>"
            } else {
                "<class 'int'>"
            },
        }),
        Value::Object(_) => Err(ContentError::InvalidContentType {
            type_name: "<class 'dict'>",
        }),
    }
}

#[must_use]
pub fn instructions_to_system_message(instructions: Option<&str>) -> Value {
    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("system".to_owned()));
    message.insert(
        "content".to_owned(),
        Value::String(instructions.unwrap_or("").to_owned()),
    );
    Value::Object(message)
}

mod tests;
