use fancy_regex::Regex;

use crate::arguments::json::{Json, Object};

const MAX_SCHEMA_NESTING: usize = 1024;

pub(super) fn drop_regexes(schema: &mut Object, depth: usize) {
    if depth >= MAX_SCHEMA_NESTING {
        return;
    }
    if schema
        .get("pattern")
        .and_then(Json::as_str)
        .is_some_and(|pattern| !is_python_regex(pattern))
    {
        schema.shift_remove("pattern");
    }
    let keys: Vec<String> = schema.keys().cloned().collect();
    for key in keys {
        let Some(value) = schema.get_mut(&key) else {
            continue;
        };
        let Some(next_depth) = depth.checked_add(1) else {
            return;
        };
        process_schema_child(value, &key, next_depth);
    }
}

fn process_schema_child(value: &mut Json, key: &str, depth: usize) {
    match key {
        "patternProperties" => {
            if let Json::Object(children) = value {
                drop_pattern_properties(children, depth);
            }
        }
        map_key if is_schema_map_key(map_key) => {
            if let Json::Object(children) = value {
                drop_map_children(children, depth);
            }
        }
        "items" => match value {
            Json::List(children) => drop_list_children(children, depth),
            Json::Object(child_schema) => drop_regexes(child_schema, depth),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        },
        list_key if is_schema_list_key(list_key) => {
            if let Json::List(children) = value {
                drop_list_children(children, depth);
            }
        }
        object_key if is_schema_object_key(object_key) => {
            if let Json::Object(child_schema) = value {
                drop_regexes(child_schema, depth);
            }
        }
        _ => {}
    }
}

fn drop_pattern_properties(children: &mut Object, depth: usize) {
    let names: Vec<String> = children.keys().cloned().collect();
    for name in names {
        if !is_python_regex(&name) {
            children.shift_remove(&name);
        }
    }
    drop_map_children(children, depth);
}

fn drop_map_children(children: &mut Object, depth: usize) {
    for value in children.values_mut() {
        if let Json::Object(child_schema) = value {
            drop_regexes(child_schema, depth);
        }
    }
}

fn drop_list_children(children: &mut [Json], depth: usize) {
    for value in children {
        if let Json::Object(child_schema) = value {
            drop_regexes(child_schema, depth);
        }
    }
}

fn is_python_regex(pattern: &str) -> bool {
    if has_unescaped(pattern, "\\p{")
        || has_unescaped(pattern, "\\P{")
        || has_unescaped(pattern, "\\z")
    {
        return false;
    }
    Regex::new(pattern).is_ok()
}

fn has_unescaped(pattern: &str, needle: &str) -> bool {
    for (index, _) in pattern.match_indices(needle) {
        let slash_count = pattern
            .bytes()
            .take(index)
            .rev()
            .take_while(|character| *character == b'\\')
            .count();
        if slash_count & 1 == 0 {
            return true;
        }
    }
    false
}

fn is_schema_map_key(key: &str) -> bool {
    let result = matches!(
        key,
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties"
    );
    result
}

fn is_schema_list_key(key: &str) -> bool {
    let result = matches!(key, "allOf" | "anyOf" | "oneOf" | "prefixItems");
    result
}

fn is_schema_object_key(key: &str) -> bool {
    let result = matches!(
        key,
        "additionalItems"
            | "additionalProperties"
            | "contains"
            | "else"
            | "if"
            | "not"
            | "propertyNames"
            | "then"
            | "unevaluatedItems"
            | "unevaluatedProperties"
    );
    result
}

mod tests;
