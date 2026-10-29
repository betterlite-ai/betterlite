use std::collections::BTreeSet;

use serde_json::Value;

pub(super) fn remove_pointer(value: &mut Value, pointer: &str) {
    if pointer.is_empty() {
        *value = Value::Null;
        return;
    }
    if let Some((parent, key)) = pointer.rsplit_once('/') {
        let decoded = key.replace("~1", "/").replace("~0", "~");
        if let Some(Value::Object(object)) = value.pointer_mut(parent) {
            object.remove(&decoded);
            return;
        }
        if let Some(child) = value.pointer_mut(pointer) {
            *child = Value::Null;
        }
    }
}

pub(super) fn first_difference(left: &Value, right: &Value, pointer: &str) -> Option<String> {
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(left_object), Value::Object(right_object)) => {
            let keys = left_object
                .keys()
                .chain(right_object.keys())
                .collect::<BTreeSet<_>>();
            keys.into_iter().find_map(|key| {
                let child = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                first_difference_optional(left_object.get(key), right_object.get(key), &child)
            })
        }
        (Value::Array(left_array), Value::Array(right_array)) => {
            let length = left_array.len().max(right_array.len());
            (0..length).find_map(|index| {
                let child = format!("{pointer}/{index}");
                first_difference_optional(left_array.get(index), right_array.get(index), &child)
            })
        }
        (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_), _)
        | (_, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        | (Value::Array(_), Value::Object(_))
        | (Value::Object(_), Value::Array(_)) => Some(if pointer.is_empty() {
            "/".to_owned()
        } else {
            pointer.to_owned()
        }),
    }
}

fn first_difference_optional(
    left: Option<&Value>,
    right: Option<&Value>,
    pointer: &str,
) -> Option<String> {
    match (left, right) {
        (Some(left_value), Some(right_value)) => first_difference(left_value, right_value, pointer),
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => Some(pointer.to_owned()),
    }
}
