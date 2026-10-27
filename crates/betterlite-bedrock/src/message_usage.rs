#![expect(
    clippy::disallowed_types,
    reason = "Bedrock message events are provider records whose arbitrary fields must be preserved"
)]

use serde_json::Value;

type Object = serde_json::Map<String, Value>;

const CACHE_FIELDS: [&str; 2] = ["cache_creation_input_tokens", "cache_read_input_tokens"];

pub fn promote_message_usage(events: Vec<Value>) -> Vec<Value> {
    let decoded = events.into_iter().map(promote_metrics).collect::<Vec<_>>();
    promote_stream(decoded)
}

fn promote_metrics(mut event: Value) -> Value {
    let Some(object) = event.as_object_mut() else {
        return event;
    };
    let metrics = object.remove("amazon-bedrock-invocationMetrics");
    let Some(Value::Object(metrics_object)) = metrics else {
        return event;
    };
    if metrics_object.is_empty() {
        return event;
    }
    let mut usage = Object::new();
    for (metric, field) in [
        ("inputTokenCount", "input_tokens"),
        ("outputTokenCount", "output_tokens"),
        ("cacheReadInputTokenCount", "cache_read_input_tokens"),
        ("cacheWriteInputTokenCount", "cache_creation_input_tokens"),
    ] {
        if let Some(value) = metrics_object.get(metric) {
            usage.insert(field.to_owned(), value.clone());
        }
    }
    if let Some(Value::Object(existing)) = object.remove("usage") {
        usage.extend(existing);
    }
    object.insert("usage".to_owned(), Value::Object(usage));
    event
}

fn promote_stream(events: Vec<Value>) -> Vec<Value> {
    let mut output = Vec::with_capacity(events.len());
    let mut pending: Option<Object> = None;
    let mut start_usage: Option<Object> = None;
    for event in events {
        let Value::Object(object) = event else {
            flush(&mut pending, &mut output, start_usage.as_ref(), false);
            output.push(event);
            continue;
        };
        match object.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(usage) = object
                    .get("message")
                    .and_then(Value::as_object)
                    .and_then(|message| message.get("usage"))
                    .and_then(Value::as_object)
                {
                    start_usage = Some(usage.clone());
                }
                flush(&mut pending, &mut output, start_usage.as_ref(), false);
                output.push(Value::Object(object));
            }
            Some("message_delta") => pending = Some(object),
            Some("message_stop") if pending.is_some() => {
                output.extend(promote_stop(object, pending.take(), start_usage.as_ref()));
            }
            _ => {
                flush(&mut pending, &mut output, start_usage.as_ref(), false);
                output.push(Value::Object(object));
            }
        }
    }
    flush(&mut pending, &mut output, start_usage.as_ref(), true);
    output
}

fn promote_stop(object: Object, pending: Option<Object>, start: Option<&Object>) -> Vec<Value> {
    let mut delta = pending.unwrap_or_default();
    let stop = object.get("usage").and_then(Value::as_object);
    let mut delta_usage = delta
        .get("usage")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    copy_stop_usage(&mut delta_usage, stop);
    merge_start(&mut delta_usage, start);
    if !delta_usage.is_empty() {
        delta.insert("usage".to_owned(), Value::Object(delta_usage));
    }
    vec![Value::Object(delta), Value::Object(object)]
}

fn copy_stop_usage(delta: &mut Object, stop_usage: Option<&Object>) {
    let Some(stop) = stop_usage else { return };
    for field in CACHE_FIELDS {
        if let Some(value) = stop.get(field) {
            delta.insert(field.to_owned(), value.clone());
        }
    }
    if let Some(value) = stop.get("input_tokens")
        && !value.is_null()
    {
        let input = if value.is_boolean() || value.is_i64() || value.is_u64() {
            value.clone()
        } else {
            Value::from(0_u64)
        };
        delta.insert("input_tokens".to_owned(), input);
    }
}

fn flush(
    pending: &mut Option<Object>,
    output: &mut Vec<Value>,
    start: Option<&Object>,
    merge: bool,
) {
    if let Some(mut delta) = pending.take() {
        if merge {
            let mut usage = delta
                .get("usage")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            merge_start(&mut usage, start);
            if !usage.is_empty() {
                delta.insert("usage".to_owned(), Value::Object(usage));
            }
        }
        output.push(Value::Object(delta));
    }
}

fn merge_start(delta: &mut Object, start_usage: Option<&Object>) {
    let Some(start_values) = start_usage else {
        return;
    };
    for field in CACHE_FIELDS {
        if !delta.contains_key(field)
            && let Some(value) = start_values.get(field)
            && !value.is_null()
        {
            delta.insert(field.to_owned(), value.clone());
        }
    }
    if !delta.contains_key("cache_creation")
        && let Some(value) = start_values.get("cache_creation")
        && !value.is_null()
    {
        delta.insert("cache_creation".to_owned(), value.clone());
    }
}

mod tests;
