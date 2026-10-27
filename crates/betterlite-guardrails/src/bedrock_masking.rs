use serde_json::Value;

pub fn extract_masked_texts(response: Option<&Value>) -> Vec<String> {
    let Some(outputs) = response.and_then(|value| value.get("outputs")) else {
        return Vec::new();
    };
    let Some(outputs) = outputs.as_array() else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    for output in outputs {
        let Some(output) = output.as_object() else {
            break;
        };
        let Some(text) = output.get("text") else {
            break;
        };
        let Some(text) = text.as_str() else { break };
        texts.push(text.to_owned());
    }
    texts
}

pub fn apply_masking_to_messages(messages: &[Value], masked_texts: &[String]) -> Vec<Value> {
    let mut index = 0;
    messages
        .iter()
        .map(|message| mask_message(message, masked_texts, &mut index))
        .collect()
}

fn mask_message(message: &Value, masked_texts: &[String], index: &mut usize) -> Value {
    let Some(object) = message.as_object() else {
        return message.clone();
    };
    let Some(content) = object.get("content") else {
        return message.clone();
    };
    let mut updated = object.clone();
    match content {
        Value::String(_) => {
            if let Some(text) = masked_texts.get(*index) {
                updated.insert("content".to_owned(), Value::String(text.clone()));
                *index += 1;
            }
        }
        Value::Array(items) => {
            updated.insert(
                "content".to_owned(),
                Value::Array(mask_content(items, masked_texts, index)),
            );
        }
        _ => {}
    }
    Value::Object(updated)
}

fn mask_content(items: &[Value], masked_texts: &[String], index: &mut usize) -> Vec<Value> {
    items
        .iter()
        .map(|item| match item {
            Value::Object(object) if object.contains_key("text") => {
                let mut updated = object.clone();
                if let Some(text) = masked_texts.get(*index) {
                    updated.insert("text".to_owned(), Value::String(text.clone()));
                    *index += 1;
                }
                Value::Object(updated)
            }
            Value::String(_) => masked_texts.get(*index).map_or_else(
                || item.clone(),
                |text| {
                    *index += 1;
                    Value::String(text.clone())
                },
            ),
            _ => item.clone(),
        })
        .collect()
}

pub fn apply_masking_to_response(response: &mut Value, masked_texts: &[String]) {
    let Some(choices) = response.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    let mut index = 0;
    for choice in choices {
        let Some(choice) = choice.as_object_mut() else {
            continue;
        };
        let locations = ["message", "delta"];
        let mut applied = false;
        for location in locations {
            if let Some(content) = choice
                .get_mut(location)
                .and_then(Value::as_object_mut)
                .and_then(|object| object.get_mut("content"))
            {
                if content.is_string() {
                    if let Some(masked) = masked_texts.get(index) {
                        *content = Value::String(masked.clone());
                        index += 1;
                        applied = true;
                    }
                }
            }
            if applied {
                break;
            }
        }
        if !applied && choice.get("text").is_some_and(Value::is_string) {
            if let Some(masked) = masked_texts.get(index) {
                choice.insert("text".to_owned(), Value::String(masked.clone()));
                index += 1;
            }
        }
    }
}
