use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type Object = BTreeMap<String, Json>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(Object),
}

impl Json {
    const fn object(&self) -> Option<&Object> {
        if let Self::Object(object) = self {
            Some(object)
        } else {
            None
        }
    }
    fn is_integer(&self) -> bool {
        match self {
            Self::Bool(_) => true,
            Self::Number(number) => number.is_i64() || number.is_u64(),
            Self::Null | Self::String(_) | Self::Array(_) | Self::Object(_) => false,
        }
    }
}

const CACHE_FIELDS: [&str; 2] = ["cache_creation_input_tokens", "cache_read_input_tokens"];

pub fn promote_message_usage(events: Vec<Json>) -> Vec<Json> {
    let decoded = events.into_iter().map(promote_metrics).collect::<Vec<_>>();
    promote_stream(decoded)
}

fn promote_metrics(mut event: Json) -> Json {
    let Some(object) = object_mut(&mut event) else {
        return event;
    };
    let metrics = object.remove("amazon-bedrock-invocationMetrics");
    let Some(Json::Object(metrics_object)) = metrics else {
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
    if let Some(Json::Object(existing)) = object.remove("usage") {
        usage.extend(existing);
    }
    object.insert("usage".to_owned(), Json::Object(usage));
    event
}

const fn object_mut(event: &mut Json) -> Option<&mut Object> {
    if let Json::Object(object) = event {
        Some(object)
    } else {
        None
    }
}

fn promote_stream(events: Vec<Json>) -> Vec<Json> {
    let mut output = Vec::with_capacity(events.len());
    let mut pending: Option<Object> = None;
    let mut start_usage: Option<Object> = None;
    for event in events {
        let Json::Object(object) = event else {
            flush(&mut pending, &mut output, start_usage.as_ref(), false);
            output.push(event);
            continue;
        };
        match object.get("type").and_then(as_string) {
            Some("message_start") => {
                if let Some(usage) = object
                    .get("message")
                    .and_then(Json::object)
                    .and_then(|message| message.get("usage"))
                    .and_then(Json::object)
                {
                    start_usage = Some(usage.clone());
                }
                flush(&mut pending, &mut output, start_usage.as_ref(), false);
                output.push(Json::Object(object));
            }
            Some("message_delta") => pending = Some(object),
            Some("message_stop") if pending.is_some() => {
                output.extend(promote_stop(object, pending.take(), start_usage.as_ref()));
            }
            _ => {
                flush(&mut pending, &mut output, start_usage.as_ref(), false);
                output.push(Json::Object(object));
            }
        }
    }
    flush(&mut pending, &mut output, start_usage.as_ref(), true);
    output
}

fn promote_stop(object: Object, pending: Option<Object>, start: Option<&Object>) -> Vec<Json> {
    let mut delta = pending.unwrap_or_default();
    let stop = object.get("usage").and_then(Json::object);
    let mut delta_usage = delta
        .get("usage")
        .and_then(Json::object)
        .cloned()
        .unwrap_or_default();
    copy_stop_usage(&mut delta_usage, stop);
    merge_start(&mut delta_usage, start);
    if !delta_usage.is_empty() {
        delta.insert("usage".to_owned(), Json::Object(delta_usage));
    }
    vec![Json::Object(delta), Json::Object(object)]
}

fn copy_stop_usage(delta: &mut Object, stop_usage: Option<&Object>) {
    let Some(stop) = stop_usage else { return };
    for field in CACHE_FIELDS {
        if let Some(value) = stop.get(field) {
            delta.insert(field.to_owned(), value.clone());
        }
    }
    if let Some(value) = stop.get("input_tokens") {
        let input = if value.is_integer() {
            value.clone()
        } else {
            Json::Number(serde_json::Number::from(0_u64))
        };
        delta.insert("input_tokens".to_owned(), input);
    }
}

fn as_string(value: &Json) -> Option<&str> {
    if let Json::String(text) = value {
        Some(text)
    } else {
        None
    }
}

fn flush(
    pending: &mut Option<Object>,
    output: &mut Vec<Json>,
    start: Option<&Object>,
    merge: bool,
) {
    if let Some(mut delta) = pending.take() {
        if merge {
            let mut usage = delta
                .get("usage")
                .and_then(Json::object)
                .cloned()
                .unwrap_or_default();
            merge_start(&mut usage, start);
            if !usage.is_empty() {
                delta.insert("usage".to_owned(), Json::Object(usage));
            }
        }
        output.push(Json::Object(delta));
    }
}

fn merge_start(delta: &mut Object, start_usage: Option<&Object>) {
    let Some(start_values) = start_usage else {
        return;
    };
    for field in CACHE_FIELDS {
        if !delta.contains_key(field)
            && let Some(value) = start_values.get(field)
            && !matches!(value, Json::Null)
        {
            delta.insert(field.to_owned(), value.clone());
        }
    }
    if !delta.contains_key("cache_creation")
        && let Some(value) = start_values.get("cache_creation")
        && !matches!(value, Json::Null)
    {
        delta.insert("cache_creation".to_owned(), value.clone());
    }
}

mod tests;
