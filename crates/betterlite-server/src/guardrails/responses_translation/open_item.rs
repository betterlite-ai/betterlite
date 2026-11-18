#![expect(
    clippy::disallowed_types,
    reason = "Responses events are open provider JSON"
)]

use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct OpenItemState {
    pub item_id: String,
    pub item_type: String,
    pub role: String,
    pub output_index: i64,
    pub content_index: i64,
    pub text: String,
    pub part_open: bool,
    pub payload: Value,
}

pub fn incomplete_item_fields(payload: &Value) -> Map<String, Value> {
    payload.as_object().cloned().unwrap_or_default()
}

pub fn open_item_state(events: &[Value]) -> Option<OpenItemState> {
    let mut added = Vec::<(i64, Value)>::new();
    let mut done = Vec::<i64>::new();
    for event in events {
        let index = event.get("output_index").and_then(Value::as_i64);
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.added") => {
                if let (Some(index), Some(item)) = (index, event.get("item")) {
                    added.push((index, item.clone()));
                }
            }
            Some("response.output_item.done") => {
                if let Some(index) = index {
                    done.push(index);
                }
            }
            _ => {}
        }
    }
    let (output_index, payload) = added
        .into_iter()
        .rev()
        .find(|(index, _)| !done.contains(index))?;
    let item_id = payload.get("id")?.as_str()?.to_owned();
    if item_id.is_empty() {
        return None;
    }
    let item_type = payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("message")
        .to_owned();
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("assistant")
        .to_owned();
    let mut parts = Vec::<i64>::new();
    let mut closed = Vec::<i64>::new();
    let mut text = String::new();
    for event in events {
        if event.get("item_id").and_then(Value::as_str) != Some(item_id.as_str()) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.content_part.added") => {
                if let Some(index) = event.get("content_index").and_then(Value::as_i64) {
                    parts.push(index);
                }
            }
            Some("response.content_part.done") => {
                if let Some(index) = event.get("content_index").and_then(Value::as_i64) {
                    closed.push(index);
                }
            }
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    text.push_str(delta);
                }
            }
            _ => {}
        }
    }
    let content_index = parts
        .iter()
        .rev()
        .find(|index| !closed.contains(index))
        .copied()
        .unwrap_or(0);
    Some(OpenItemState {
        item_id,
        item_type,
        role,
        output_index,
        content_index,
        text,
        part_open: parts.iter().any(|index| !closed.contains(index)),
        payload,
    })
}

pub fn open_item_closing_events(events: &[Value]) -> Vec<Value> {
    let Some(state) = open_item_state(events) else {
        return Vec::new();
    };
    if state.item_type != "message" {
        let mut item = incomplete_item_fields(&state.payload);
        item.insert("status".into(), json!("incomplete"));
        return vec![
            json!({"type":"response.output_item.done","output_index":state.output_index,"item":item}),
        ];
    }
    let mut result = Vec::new();
    if state.part_open {
        result.push(json!({"type":"response.output_text.done","item_id":state.item_id,"output_index":state.output_index,"content_index":state.content_index,"text":state.text}));
        result.push(json!({"type":"response.content_part.done","item_id":state.item_id,"output_index":state.output_index,"content_index":state.content_index,"part":{"type":"output_text","text":state.text,"annotations":[],"logprobs":null}}));
    }
    result.push(json!({"type":"response.output_item.done","output_index":state.output_index,"item":{"id":state.item_id,"type":"message","status":"completed","role":state.role,"content":[{"type":"output_text","text":state.text,"annotations":[]} ]}}));
    result
}

#[cfg(test)]
mod tests;
