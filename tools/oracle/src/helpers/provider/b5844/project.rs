use serde_json::{Value, json};

// A missing field reads as null here, as the Python views declare it nullable or never leave it out
fn usage(value: &Value) -> Value {
    json!({
        "prompt_tokens": value["prompt_tokens"],
        "completion_tokens": value["completion_tokens"],
        "total_tokens": value["total_tokens"],
    })
}

fn tool_view(call: &Value, with_index: bool) -> Value {
    let mut view = json!({
        "id": call["id"],
        "type": call["type"],
        "function": {
            "name": call["function"]["name"],
            "arguments": call["function"]["arguments"],
        },
    });
    if with_index {
        view["index"] = call["index"].clone();
    }
    view
}

fn calls(value: &Value, with_index: bool) -> Value {
    let views = value.as_array().map(|items| {
        items
            .iter()
            .map(|call| tool_view(call, with_index))
            .collect()
    });
    views.map_or(Value::Null, Value::Array)
}

pub(crate) fn reply(value: &Value) -> Value {
    let mut choices = Vec::new();
    for item in value["choices"].as_array().into_iter().flatten() {
        choices.push(json!({
            "index": item["index"],
            "finish_reason": item["finish_reason"],
            "message": {
                "role": item["message"]["role"],
                "content": item["message"]["content"],
                "tool_calls": calls(&item["message"]["tool_calls"], false),
            },
        }));
    }
    json!({
        "id": value["id"],
        "model": value["model"],
        "choices": choices,
        "usage": usage(&value["usage"]),
    })
}

pub(crate) fn embedding(value: &Value) -> Value {
    let mut data = Vec::new();
    for item in value["data"].as_array().into_iter().flatten() {
        data.push(json!({"index": item["index"], "embedding": item["embedding"]}));
    }
    json!({
        "model": value["model"],
        "data": data,
        "usage": usage(&value["usage"]),
    })
}

pub(crate) fn event(value: &Value) -> Value {
    let mut choices = Vec::new();
    for item in value["choices"].as_array().into_iter().flatten() {
        choices.push(json!({
            "index": item["index"],
            "finish_reason": item["finish_reason"],
            "delta": {
                "content": item["delta"]["content"],
                "tool_calls": calls(&item["delta"]["tool_calls"], true),
            },
        }));
    }
    let counted = if value["usage"].is_null() {
        Value::Null
    } else {
        usage(&value["usage"])
    };
    json!({"model": value["model"], "choices": choices, "usage": counted})
}
