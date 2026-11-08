use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::estimate::estimate_json_bytes;
use betterlite_prompts::definitions::legacy::DEFS_MAX_INLINED_BYTES;

use crate::oci_common::OciError;

const MAX_SCHEMA_DEPTH: usize = 256;

pub const OCI_JSON_TO_PYTHON_TYPES: [(&str, &str); 7] = [
    ("string", "str"),
    ("number", "float"),
    ("boolean", "bool"),
    ("integer", "int"),
    ("array", "List"),
    ("object", "Dict"),
    ("any", "any"),
];

struct Walk<'defs> {
    defs: Option<&'defs Object>,
    resolving: BTreeSet<String>,
    spent: usize,
}

pub fn resolve_oci_schema_refs(schema: &Json) -> Result<Json, OciError> {
    let defs = schema
        .as_object()
        .and_then(|object| object.get("$defs"))
        .and_then(Json::as_object);
    let mut walk = Walk {
        defs,
        resolving: BTreeSet::new(),
        spent: 0,
    };
    let mut resolved = resolve_refs(schema, &mut walk, 0)?;
    if let Json::Object(fields) = &mut resolved {
        fields.shift_remove("$defs");
    }
    Ok(resolved)
}

fn resolve_refs(value: &Json, walk: &mut Walk<'_>, depth: usize) -> Result<Json, OciError> {
    if depth >= MAX_SCHEMA_DEPTH {
        return Ok(Json::Object(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )])));
    }
    let next = depth.saturating_add(1);
    let fields = match value {
        Json::Object(fields) => fields,
        Json::List(items) => return resolve_list(items, walk, next),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return Ok(value.clone()),
    };
    match fields.get("$ref") {
        None | Some(Json::Null) => {}
        Some(Json::Text(reference)) if reference.starts_with("#/$defs/") => {
            let resolved = resolve_reference(value, reference, walk, depth)?;
            return Ok(resolved.unwrap_or_else(|| {
                Json::Object(Object::from([(
                    "type".to_owned(),
                    Json::Text("object".to_owned()),
                )]))
            }));
        }
        Some(_) => return Ok(value.clone()),
    }
    resolve_fields(fields, walk, next)
}

fn resolve_list(items: &[Json], walk: &mut Walk<'_>, next: usize) -> Result<Json, OciError> {
    let resolved = items
        .iter()
        .map(|item| resolve_refs(item, walk, next))
        .collect::<Result<Vec<Json>, OciError>>()?;
    Ok(Json::List(resolved))
}

fn resolve_fields(fields: &Object, walk: &mut Walk<'_>, next: usize) -> Result<Json, OciError> {
    let mut resolved = Object::new();
    for (key, field) in fields {
        resolved.insert(key.clone(), resolve_refs(field, walk, next)?);
    }
    Ok(Json::Object(resolved))
}

fn resolve_reference(
    value: &Json,
    reference: &str,
    walk: &mut Walk<'_>,
    depth: usize,
) -> Result<Option<Json>, OciError> {
    let key = reference.rsplit('/').next().unwrap_or_default().to_owned();
    if !walk.resolving.insert(key.clone()) {
        return Ok(None);
    }
    let target = walk.defs.and_then(|table| table.get(&key)).unwrap_or(value);
    walk.spent = walk.spent.saturating_add(estimate_json_bytes(target));
    if walk.spent > DEFS_MAX_INLINED_BYTES {
        return Err(OciError {
            status: 400,
            message: format!(
                "the OCI tool schema expands past {DEFS_MAX_INLINED_BYTES} estimated bytes"
            ),
        });
    }
    let resolved = resolve_refs(target, walk, depth.saturating_add(1));
    walk.resolving.remove(&key);
    resolved.map(Some)
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
            "properties" => {
                if let Json::Object(properties) = value {
                    sanitized.insert(key.clone(), Json::Object(sanitize_properties(properties)));
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

fn sanitize_properties(properties: &Object) -> Object {
    properties
        .iter()
        .map(|(name, schema)| (name.clone(), sanitize_oci_schema(schema)))
        .collect()
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
