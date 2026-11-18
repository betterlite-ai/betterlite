#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Responses stream events are provider JSON kept so the still-open item can be closed"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        reason = "serde_json fixtures compare JSON numbers and construct provider objects"
    )
)]

use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[must_use]
pub fn incomplete_item_fields(payload: &Value) -> Map<String, Value> {
    match payload {
        Value::Object(fields) => fields.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Map::new()
        }
    }
}

fn open_added_item(responses_so_far: &[Value]) -> Option<(i64, &Value)> {
    let mut added = Vec::<(i64, Option<&Value>)>::new();
    let mut done = Vec::<i64>::new();
    for event in responses_so_far {
        let index = event.get("output_index").and_then(Value::as_i64);
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.added") => {
                if let Some(added_index) = index {
                    added.push((added_index, event.get("item")));
                }
            }
            Some("response.output_item.done") => {
                if let Some(done_index) = index {
                    done.push(done_index);
                }
            }
            _ => {}
        }
    }
    let (output_index, payload) = added
        .into_iter()
        .rev()
        .find(|(index, _)| !done.contains(index))?;
    payload.map(|item| (output_index, item))
}

fn open_part_state(responses_so_far: &[Value], item_id: &str) -> (i64, bool, String) {
    let mut parts = Vec::<i64>::new();
    let mut closed = Vec::<i64>::new();
    let mut text = String::new();
    for event in responses_so_far {
        if event.get("item_id").and_then(Value::as_str) != Some(item_id) {
            continue;
        }
        let index = event.get("content_index").and_then(Value::as_i64);
        match event.get("type").and_then(Value::as_str) {
            Some("response.content_part.added") => {
                if let Some(added_index) = index {
                    parts.push(added_index);
                }
            }
            Some("response.content_part.done") => {
                if let Some(done_index) = index {
                    closed.push(done_index);
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
    let part_open = parts.iter().any(|index| !closed.contains(index));
    (content_index, part_open, text)
}

fn non_empty_field<'payload>(payload: &'payload Value, name: &str) -> Option<&'payload str> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

#[must_use]
pub fn open_item_state(responses_so_far: &[Value]) -> Option<OpenItemState> {
    let (output_index, payload) = open_added_item(responses_so_far)?;
    let item_id = payload.get("id").and_then(Value::as_str)?;
    if item_id.is_empty() {
        return None;
    }
    let (content_index, part_open, text) = open_part_state(responses_so_far, item_id);
    Some(OpenItemState {
        item_id: item_id.to_owned(),
        item_type: non_empty_field(payload, "type")
            .unwrap_or("message")
            .to_owned(),
        role: non_empty_field(payload, "role")
            .unwrap_or("assistant")
            .to_owned(),
        output_index,
        content_index,
        text,
        part_open,
        payload: payload.clone(),
    })
}

fn message_closing_events(state: &OpenItemState) -> Vec<Value> {
    let mut events = Vec::new();
    if state.part_open {
        events.push(json!({
            "type": "response.output_text.done",
            "item_id": state.item_id,
            "output_index": state.output_index,
            "content_index": state.content_index,
            "text": state.text,
        }));
        events.push(json!({
            "type": "response.content_part.done",
            "item_id": state.item_id,
            "output_index": state.output_index,
            "content_index": state.content_index,
            "part": {"type": "output_text", "text": state.text, "annotations": [], "logprobs": null},
        }));
    }
    events.push(json!({
        "type": "response.output_item.done",
        "output_index": state.output_index,
        "item": {
            "id": state.item_id,
            "type": "message",
            "status": "completed",
            "role": state.role,
            "content": [{"type": "output_text", "text": state.text, "annotations": []}],
        },
    }));
    events
}

#[must_use]
pub fn open_item_closing_events(responses_so_far: &[Value]) -> Vec<Value> {
    let Some(state) = open_item_state(responses_so_far) else {
        return Vec::new();
    };
    if state.item_type != "message" {
        let mut item = incomplete_item_fields(&state.payload);
        item.insert("status".to_owned(), json!("incomplete"));
        return vec![json!({
            "type": "response.output_item.done",
            "output_index": state.output_index,
            "item": item,
        })];
    }
    message_closing_events(&state)
}

mod tests;
