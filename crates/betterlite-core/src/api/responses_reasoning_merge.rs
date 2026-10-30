use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub thinking_blocks: Option<Vec<Value>>,
    #[serde(default)]
    pub tool_calls: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("assistant content must be a string or a list of text blocks")]
    InvalidContent,
}

fn field<'a>(message: &'a Value, name: &str) -> Option<&'a Value> {
    let fields = message.as_object()?;
    fields.get(name)
}
fn text(message: &Value) -> Option<&str> {
    let value = field(message, "reasoning_content")?.as_str()?;
    (!value.is_empty()).then_some(value)
}
fn blocks(message: &Value) -> Option<&[Value]> {
    let blocks = field(message, "thinking_blocks")?.as_array()?;
    (!blocks.is_empty()).then_some(blocks.as_slice())
}
fn pending(message: &Value) -> bool {
    let role = field(message, "role").and_then(Value::as_str) == Some("assistant");
    let content = field(message, "content").is_some_and(Value::is_null);
    let tools = !field(message, "tool_calls")
        .is_some_and(|value| !value.is_null() && value != &Value::Array(Vec::new()));
    let reasoning = text(message).is_some() || blocks(message).is_some();
    role && content && tools && reasoning
}
fn standalone(texts: &[Option<&str>], block_lists: &[Option<&[Value]>]) -> Value {
    let mut result = Map::from([
        ("role".to_owned(), Value::String("assistant".to_owned())),
        ("content".to_owned(), Value::Null),
    ]);
    let joined = texts
        .iter()
        .filter_map(|value| *value)
        .collect::<Vec<_>>()
        .join("\n");
    if !joined.is_empty() {
        result.insert("reasoning_content".to_owned(), Value::String(joined));
    }
    let merged = block_lists
        .iter()
        .flat_map(|value| value.iter().flat_map(|items| items.iter().cloned()))
        .collect::<Vec<_>>();
    if !merged.is_empty() {
        result.insert("thinking_blocks".to_owned(), Value::Array(merged));
    }
    Value::Object(result)
}
fn apply_pending(message: &mut Value, texts: &[Option<&str>], block_lists: &[Option<&[Value]>]) {
    let mut all_texts = texts.iter().filter_map(|value| *value).collect::<Vec<_>>();
    if let Some(value) = text(message) {
        all_texts.push(value);
    }
    if !all_texts.is_empty() {
        if let Some(fields) = message.as_object_mut() {
            fields.insert(
                "reasoning_content".to_owned(),
                Value::String(all_texts.join("\n")),
            );
        }
    }
    let mut all_blocks = block_lists
        .iter()
        .flat_map(|value| value.iter().flat_map(|items| items.iter().cloned()))
        .collect::<Vec<_>>();
    if let Some(value) = blocks(message) {
        all_blocks.extend(value.iter().cloned());
    }
    if !all_blocks.is_empty() {
        if let Some(fields) = message.as_object_mut() {
            fields.insert("thinking_blocks".to_owned(), Value::Array(all_blocks));
        }
    }
}

pub fn merge_reasoning(messages: &[Value]) -> Vec<Value> {
    let mut merged = Vec::with_capacity(messages.len());
    let mut texts = Vec::new();
    let mut block_lists = Vec::new();
    for message in messages {
        if pending(message) {
            texts.push(text(message));
            block_lists.push(blocks(message));
            continue;
        }
        if !texts.is_empty() || !block_lists.is_empty() {
            if field(message, "role").and_then(Value::as_str) == Some("assistant") {
                let mut target = message.clone();
                apply_pending(&mut target, &texts, &block_lists);
                merged.push(target);
            } else {
                for (text, blocks) in texts.iter().zip(block_lists.iter()) {
                    merged.push(standalone(&[*text], &[*blocks]));
                }
                merged.push(message.clone());
            }
            texts.clear();
            block_lists.clear();
        } else {
            merged.push(message.clone());
        }
    }
    for (text, blocks) in texts.iter().zip(block_lists.iter()) {
        merged.push(standalone(&[*text], &[*blocks]));
    }
    merged
}

pub fn merge_chat_records(messages: &[ChatRecord]) -> Result<Vec<ChatRecord>, serde_json::Error> {
    let values = messages
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    merge_reasoning(&values)
        .into_iter()
        .map(serde_json::from_value)
        .collect()
}
fn content_blocks(value: &Value) -> Result<Vec<Value>, MergeError> {
    match value {
        Value::String(text) => Ok(vec![serde_json::json!({"type": "text", "text": text})]),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let fields = item.as_object().ok_or(MergeError::InvalidContent)?;
                if fields.get("type").and_then(Value::as_str) != Some("text")
                    || fields.get("text").and_then(Value::as_str).is_none()
                {
                    return Err(MergeError::InvalidContent);
                }
                Ok(item.clone())
            })
            .collect(),
        _ => Err(MergeError::InvalidContent),
    }
}
pub fn merge_trailing(
    messages: &[Value],
    new_messages: &[Value],
    hosted_search: bool,
) -> Result<Option<Value>, MergeError> {
    if messages.is_empty() || new_messages.len() != 1 {
        return Ok(None);
    }
    let Some(last) = messages.last().and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(new) = new_messages.first().and_then(Value::as_object) else {
        return Ok(None);
    };
    if last.get("role").and_then(Value::as_str) != Some("assistant")
        || new.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Ok(None);
    }
    if (!last
        .get("tool_calls")
        .is_some_and(|value| value != &Value::Null && value != &Value::Array(Vec::new()))
        && !hosted_search)
        || new
            .get("tool_calls")
            .is_some_and(|value| value != &Value::Null && value != &Value::Array(Vec::new()))
        || new.get("content").is_none_or(Value::is_null)
    {
        return Ok(None);
    }
    let new_content = new.get("content").ok_or(MergeError::InvalidContent)?;
    let previous = last.get("content").unwrap_or(&Value::Null);
    let content = if previous.is_null()
        || previous == &Value::Bool(false)
        || previous == &Value::Number(0.into())
        || previous == &Value::String(String::new())
        || previous == &Value::Array(Vec::new())
    {
        new_content.clone()
    } else {
        let mut blocks = content_blocks(previous)?;
        blocks.extend(content_blocks(new_content)?);
        Value::Array(blocks)
    };
    let mut result = last.clone();
    result.insert("content".to_owned(), content);
    Ok(Some(Value::Object(result)))
}

mod tests;
