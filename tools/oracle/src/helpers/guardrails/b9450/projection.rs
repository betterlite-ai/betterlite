use betterlite_guardrails::cisco_stream_scan::StreamItem;
use betterlite_models::decisions::Json as ContentJson;
use betterlite_streaming::streamed_response_assembly::records::{Choice, Chunk};
use serde_json::{Map, Value, json};

pub(super) fn stream_item(item: &StreamItem) -> Value {
    match item {
        StreamItem::Chunk(chunk) => chunk_item(chunk),
        StreamItem::Sse(frame) => sse_item(frame),
    }
}

fn chunk_item(chunk: &Chunk) -> Value {
    let choices: Vec<Value> = chunk.choices.iter().map(choice_item).collect();
    json!({
        "kind": "chunk",
        "id": chunk.id,
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "choices": choices,
    })
}

// The delta's role and content are the only choice fields both sides of the oracle see the same way.
fn choice_item(choice: &Choice) -> Value {
    let role = choice.delta.as_ref().and_then(|delta| delta.role.clone());
    let content = choice
        .delta
        .as_ref()
        .and_then(|delta| delta.content.clone());
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "role": role,
        "content": content,
    })
}

// The frames carry the same JSON in LiteLLM's spacing and ours, so the body goes out parsed.
fn sse_item(frame: &str) -> Value {
    let body = frame
        .strip_prefix("data: ")
        .and_then(|rest| rest.strip_suffix("\n\n"))
        .unwrap_or(frame);
    let parsed =
        serde_json::from_str::<Value>(body).unwrap_or_else(|_| Value::String(frame.to_owned()));
    json!({"kind": "sse", "body": parsed})
}

// The text is the block payload in LiteLLM's spaced json.dumps and our compact serde, so it is parsed.
pub(super) fn content_item(item: &ContentJson) -> Value {
    match item {
        ContentJson::Object(fields) => {
            let mut projected = Map::new();
            for (key, value) in fields {
                let rendered = if key == "text" {
                    text_field(value)
                } else {
                    json_value(value)
                };
                projected.insert(key.clone(), rendered);
            }
            Value::Object(projected)
        }
        other @ (ContentJson::Null
        | ContentJson::Bool(_)
        | ContentJson::Number(_)
        | ContentJson::Text(_)
        | ContentJson::List(_)) => json_value(other),
    }
}

fn text_field(value: &ContentJson) -> Value {
    let ContentJson::Text(text) = value else {
        return json_value(value);
    };
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
}

fn json_value(value: &ContentJson) -> Value {
    match value {
        ContentJson::Null => Value::Null,
        ContentJson::Bool(flag) => Value::Bool(*flag),
        ContentJson::Number(number) => Value::Number(number.clone()),
        ContentJson::Text(text) => Value::String(text.clone()),
        ContentJson::List(items) => Value::Array(items.iter().map(json_value).collect()),
        ContentJson::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), json_value(item)))
                .collect(),
        ),
    }
}
