#![cfg(test)]

use super::*;

fn nest(levels: u32, leaf_key: &str) -> Object {
    if levels == 0 {
        return Object::from_iter([(leaf_key.to_owned(), Json::Text("value".to_owned()))]);
    }
    Object::from_iter([(
        "child".to_owned(),
        Json::Object(nest(levels.saturating_sub(1), leaf_key)),
    )])
}

fn leaf_present_at(object: &Object, levels: u32, leaf_key: &str) -> bool {
    if levels == 0 {
        return object.contains_key(leaf_key);
    }
    match object.get("child") {
        Some(Json::Object(child)) => leaf_present_at(child, levels.saturating_sub(1), leaf_key),
        _ => false,
    }
}

mod bounds_recursion_depth;
mod removes_key_from_nested_structures;
