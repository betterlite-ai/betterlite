use serde_json::{Map, Value};

pub const MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS: usize = 8;

pub fn split_concatenated_json_objects(raw: &str) -> Vec<Map<String, Value>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
    let mut objects = Vec::new();
    while let Some(item) = stream.next() {
        match item {
            Ok(Value::Object(object)) => objects.push(object),
            Ok(_) => objects.push(Map::new()),
            Err(_) => break,
        }
    }
    objects
}

pub fn salvage_concatenated_tool_arguments(raw: &str) -> Vec<Map<String, Value>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
    let mut count: usize = 0;
    let mut kept: Vec<Map<String, Value>> = Vec::new();
    let mut exceeded = false;
    while let Some(item) = stream.next() {
        let Ok(Value::Object(object)) = item else {
            return Vec::new();
        };
        if exceeded {
            continue;
        }
        count += 1;
        keep_object(&mut kept, &mut exceeded, count, object);
    }
    if exceeded { Vec::new() } else { kept }
}

fn keep_object(
    kept: &mut Vec<Map<String, Value>>,
    exceeded: &mut bool,
    count: usize,
    object: Map<String, Value>,
) {
    if kept.is_empty() {
        kept.push(object);
        return;
    }
    if kept.len() == 1 && object == kept[0] {
        return;
    }
    if kept.len() == 1 && count > 2 {
        if count - 1 > MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS {
            *exceeded = true;
            return;
        }
        *kept = vec![kept[0].clone(); count - 1];
    }
    if kept.len() >= MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS {
        *exceeded = true;
        return;
    }
    kept.push(object);
}

mod tests;
