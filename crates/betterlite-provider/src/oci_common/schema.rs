use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};

const MAX_REF_DEPTH: usize = 32;

pub const OCI_JSON_TO_PYTHON_TYPES: [(&str, &str); 7] = [
    ("string", "str"),
    ("number", "float"),
    ("boolean", "bool"),
    ("integer", "int"),
    ("array", "List"),
    ("object", "Dict"),
    ("any", "any"),
];

#[must_use]
pub fn resolve_oci_schema_refs(schema: &Json) -> Json {
    let definitions = schema
        .as_object()
        .and_then(|object| object.get("$defs"))
        .and_then(Json::as_object);
    let mut resolving = BTreeSet::new();
    let mut resolved = resolve_refs(schema, definitions, &mut resolving, 0);
    if let Json::Object(fields) = &mut resolved {
        fields.shift_remove("$defs");
    }
    resolved
}

fn resolve_refs(
    value: &Json,
    definitions: Option<&Object>,
    resolving: &mut BTreeSet<String>,
    depth: usize,
) -> Json {
    let fields = match value {
        Json::Object(fields) => fields,
        Json::List(items) => {
            return Json::List(
                items
                    .iter()
                    .map(|item| resolve_refs(item, definitions, resolving, depth))
                    .collect(),
            );
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return value.clone(),
    };
    match fields.get("$ref") {
        None | Some(Json::Null) => {}
        Some(Json::Text(reference)) if reference.starts_with("#/$defs/") => {
            return resolve_reference(value, reference, definitions, resolving, depth);
        }
        Some(_) => return value.clone(),
    }
    Json::Object(
        fields
            .iter()
            .map(|(key, field)| {
                (
                    key.clone(),
                    resolve_refs(field, definitions, resolving, depth),
                )
            })
            .collect(),
    )
}

fn resolve_reference(
    value: &Json,
    reference: &str,
    definitions: Option<&Object>,
    resolving: &mut BTreeSet<String>,
    depth: usize,
) -> Json {
    if depth >= MAX_REF_DEPTH {
        return Json::Object(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )]));
    }
    let key = reference.rsplit('/').next().unwrap_or_default().to_owned();
    if !resolving.insert(key.clone()) {
        return Json::Object(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )]));
    }
    let target = definitions
        .and_then(|table| table.get(&key))
        .unwrap_or(value);
    let resolved = resolve_refs(target, definitions, resolving, depth.saturating_add(1));
    resolving.remove(&key);
    resolved
}

#[must_use]
pub fn resolve_oci_schema_anyof(value: &Json) -> Json {
    let fields = match value {
        Json::Object(fields) => fields,
        Json::List(items) => {
            return Json::List(items.iter().map(resolve_oci_schema_anyof).collect());
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return value.clone(),
    };
    if let Some(any_of) = fields.get("anyOf")
        && !fields.contains_key("type")
        && let Some(merged) = merged_branch(fields, any_of)
    {
        return resolve_oci_schema_anyof(&merged);
    }
    Json::Object(
        fields
            .iter()
            .map(|(key, field)| (key.clone(), resolve_oci_schema_anyof(field)))
            .collect(),
    )
}

fn merged_branch(fields: &Object, any_of: &Json) -> Option<Json> {
    let branches = match any_of {
        Json::List(items) => items.as_slice(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => &[],
    };
    let first = branches.iter().find(|branch| !is_null_branch(branch))?;
    let mut merged = fields.clone();
    if let Json::Object(branch_fields) = first {
        for (key, field) in branch_fields {
            merged.insert(key.clone(), field.clone());
        }
    }
    merged.shift_remove("anyOf");
    Some(Json::Object(merged))
}

fn is_null_branch(branch: &Json) -> bool {
    match branch {
        Json::Object(fields) => {
            matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "null")
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

#[must_use]
pub fn sanitize_oci_schema(value: &Json) -> Json {
    match value {
        Json::List(items) => Json::List(items.iter().map(sanitize_oci_schema).collect()),
        Json::Object(fields) => Json::Object(sanitize_object(fields)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

fn sanitize_object(fields: &Object) -> Object {
    let mut sanitized = Object::new();
    for (key, value) in fields {
        match key.as_str() {
            "title" => continue,
            "default" if matches!(value, Json::Null) => continue,
            "type" => {
                if let Some(normalized) = normalize_type(value) {
                    sanitized.insert(key.clone(), normalized);
                    continue;
                }
            }
            _ => {}
        }
        sanitized.insert(key.clone(), sanitize_oci_schema(value));
    }
    if matches!(sanitized.get("type"), Some(Json::Text(kind)) if kind == "array")
        && !sanitized.contains_key("items")
    {
        sanitized.insert(
            "items".to_owned(),
            Json::Object(Object::from([(
                "type".to_owned(),
                Json::Text("object".to_owned()),
            )])),
        );
    }
    filter_required(&mut sanitized);
    sanitized
}

fn normalize_type(value: &Json) -> Option<Json> {
    match value {
        Json::Text(kind) if kind == "any" => Some(Json::Text("object".to_owned())),
        Json::List(items) => Some(
            items
                .iter()
                .find(|item| !matches!(item, Json::Text(kind) if kind == "null"))
                .cloned()
                .unwrap_or_else(|| Json::Text("string".to_owned())),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

fn filter_required(sanitized: &mut Object) {
    let Some(required) = sanitized.get("required") else {
        return;
    };
    let Json::List(names) = required else {
        sanitized.insert("required".to_owned(), Json::List(Vec::new()));
        return;
    };
    let Some(properties) = sanitized.get("properties").and_then(Json::as_object) else {
        return;
    };
    let filtered = names
        .iter()
        .filter(|name| match name {
            Json::Text(text) => properties.contains_key(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
        })
        .cloned()
        .collect();
    sanitized.insert("required".to_owned(), Json::List(filtered));
}

#[must_use]
pub fn enrich_cohere_description(description: &str, schema: &Object) -> String {
    let mut parts = Vec::new();
    if !description.is_empty() {
        parts.push(description.to_owned());
    }
    if let Some(allowed) = schema.get("enum") {
        parts.push(format!("Allowed values: {}", repr_text(allowed)));
    }
    if let Some(format_value) = schema.get("format") {
        parts.push(format!("Format: {}", text_value(format_value)));
    }
    if schema.contains_key("minimum") || schema.contains_key("maximum") {
        let mut bounds = Vec::new();
        if let Some(minimum) = schema.get("minimum") {
            bounds.push(format!("min={}", text_value(minimum)));
        }
        if let Some(maximum) = schema.get("maximum") {
            bounds.push(format!("max={}", text_value(maximum)));
        }
        parts.push(format!("Range: {}", bounds.join(", ")));
    }
    if let Some(pattern) = schema.get("pattern") {
        parts.push(format!("Pattern: {}", text_value(pattern)));
    }
    parts.join(". ")
}

fn text_value(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            repr_text(value)
        }
    }
}

fn repr_text(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => quoted_text(text),
        Json::List(items) => {
            let rendered = items.iter().map(repr_text).collect::<Vec<String>>();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(key, field)| format!("{}: {}", quoted_text(key), repr_text(field)))
                .collect::<Vec<String>>();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

fn quoted_text(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut escaped = String::with_capacity(text.len());
    for symbol in text.chars() {
        match symbol {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            matching if matching == quote => {
                escaped.push('\\');
                escaped.push(matching);
            }
            plain => escaped.push(plain),
        }
    }
    format!("{quote}{escaped}{quote}")
}
