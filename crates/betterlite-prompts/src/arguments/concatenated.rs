use super::json::{Json, Object};

pub const MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS: usize = 8;

#[must_use]
pub fn split_concatenated_json_objects(raw: &str) -> Vec<Object> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Json>();
    let mut objects = Vec::new();
    for item in stream {
        match item {
            Ok(Json::Object(object)) => objects.push(object),
            Ok(_) => objects.push(Object::new()),
            Err(_) => break,
        }
    }
    objects
}

#[must_use]
pub fn salvage_concatenated_tool_arguments(raw: &str) -> Vec<Object> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Json>();
    let mut count: usize = 0;
    let mut kept: Vec<Object> = Vec::new();
    let mut exceeded = false;
    for item in stream {
        let Ok(Json::Object(object)) = item else {
            return Vec::new();
        };
        if exceeded {
            continue;
        }
        count = count.saturating_add(1);
        keep_object(&mut kept, &mut exceeded, count, object);
    }
    if exceeded { Vec::new() } else { kept }
}

fn keep_object(kept: &mut Vec<Object>, exceeded: &mut bool, count: usize, object: Object) {
    if kept.is_empty() {
        kept.push(object);
        return;
    }
    if kept.len() == 1 {
        if kept.first() == Some(&object) {
            return;
        }
        if count > 2 {
            let previous_count = count.saturating_sub(1);
            if previous_count > MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS {
                *exceeded = true;
                return;
            }
            let first = kept.first().cloned().unwrap_or_default();
            *kept = vec![first; previous_count];
        }
    }
    if kept.len() >= MAX_SALVAGED_TOOL_ARGUMENT_OBJECTS {
        *exceeded = true;
        return;
    }
    kept.push(object);
}

mod tests;
