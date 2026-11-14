use serde_json::{Value, json};

// Keeps the keys the Python views declare; a key the value lacks reads as null, and an object or list shape over a non-matching value is null
fn project(value: &Value, shape: &Value) -> Value {
    match (shape, value) {
        (Value::Object(fields), Value::Object(_)) => Value::Object(
            fields
                .iter()
                .map(|(key, inner)| (key.clone(), project(&value[key.as_str()], inner)))
                .collect(),
        ),
        (Value::Array(_), Value::Array(elements)) => elements
            .iter()
            .map(|element| project(element, &shape[0_usize]))
            .collect(),
        (Value::Object(_) | Value::Array(_), _) => Value::Null,
        (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_), _) => value.clone(),
    }
}

fn usage_shape() -> Value {
    json!({
        "prompt_tokens": null,
        "completion_tokens": null,
        "total_tokens": null,
    })
}

// A reply choice holds a message; a stream event choice holds a delta whose tool calls carry an index
pub(crate) fn choices(value: &Value, streamed: bool) -> Value {
    let mut tool_call = json!({
        "id": null,
        "type": null,
        "function": {"name": null, "arguments": null},
    });
    let mut choice = json!({"index": null, "finish_reason": null});
    let mut shape = json!({"model": null, "usage": usage_shape()});
    if streamed {
        tool_call["index"] = Value::Null;
        choice["delta"] = json!({"content": null, "tool_calls": [tool_call]});
    } else {
        choice["message"] = json!({"role": null, "content": null, "tool_calls": [tool_call]});
        shape["id"] = Value::Null;
    }
    shape["choices"] = json!([choice]);
    project(value, &shape)
}

pub(crate) fn embedding(value: &Value) -> Value {
    let item = json!({"index": null, "embedding": null});
    project(
        value,
        &json!({"model": null, "data": [item], "usage": usage_shape()}),
    )
}
