use std::fmt;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::error::DefinitionsError;
use betterlite_prompts::definitions::estimate::estimate_json_bytes;

pub mod mapping;
mod notes;
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("schema exceeds the depth or byte limit")]
    Limit,
    #[error("response format is missing type")]
    MissingType,
    #[error("schema has an invalid {0}")]
    Invalid(&'static str),
    #[error("schema definition expansion failed")]
    Definitions(#[from] DefinitionsError),
    #[error("schema note could not be serialized")]
    Encode(#[from] serde_json::Error),
    #[error("schema note could not be formatted")]
    Format(#[from] fmt::Error),
}

pub fn filter_anthropic_output_schema(input: Json) -> Result<Json, SchemaError> {
    check_bounds(&input)?;
    filter(input)
}

fn check_bounds(input: &Json) -> Result<(), SchemaError> {
    let mut pending = vec![(input, 0_u16)];
    while let Some((node, depth)) = pending.pop() {
        if depth > 128 {
            return Err(SchemaError::Limit);
        }
        match node {
            Json::Object(fields) => pending.extend(
                fields
                    .values()
                    .map(|child| (child, depth.saturating_add(1))),
            ),
            Json::List(items) => {
                pending.extend(items.iter().map(|child| (child, depth.saturating_add(1))));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    if estimate_json_bytes(input) > 10_000_000 {
        return Err(SchemaError::Limit);
    }
    Ok(())
}

fn filter(input: Json) -> Result<Json, SchemaError> {
    let Json::Object(schema) = input else {
        return Ok(input);
    };
    let note = notes::constraint_note(&schema)?;
    let drop_type = enum_type_conflict(&schema)?;
    let mut result = Object::new();
    if let Some(note_text) = &note {
        let description = match schema.get("description").filter(|value| value.is_truthy()) {
            Some(Json::Text(text)) => format!("{text} {note_text}"),
            Some(_) => return Err(SchemaError::Invalid("description")),
            None => note_text.clone(),
        };
        result.insert("description".to_owned(), Json::Text(description));
    }
    for (key, value) in schema {
        filter_field(&mut result, key, value, note.as_ref(), drop_type)?;
    }
    if result.get("type").and_then(Json::as_str) == Some("object") {
        result.insert("additionalProperties".to_owned(), Json::Bool(false));
    }
    Ok(Json::Object(result))
}

fn filter_field(
    result: &mut Object,
    key: String,
    value: Json,
    note: Option<&String>,
    drop_type: bool,
) -> Result<(), SchemaError> {
    if notes::CONSTRAINTS.iter().any(|(name, _)| *name == key)
        || (key == "type" && drop_type)
        || (key == "description" && note.is_some())
    {
        return Ok(());
    }
    let filtered = match (key.as_str(), value) {
        ("properties" | "$defs", Json::Object(fields)) => Json::Object(
            fields
                .into_iter()
                .map(|(name, child)| Ok((name, filter(child)?)))
                .collect::<Result<Object, SchemaError>>()?,
        ),
        ("items", child @ Json::Object(_)) => filter(child)?,
        ("anyOf" | "allOf" | "oneOf", Json::List(items)) => {
            return add_alternative(result, key, items);
        }
        (_, other) => other,
    };
    result.insert(key, filtered);
    Ok(())
}

fn add_alternative(result: &mut Object, key: String, items: Vec<Json>) -> Result<(), SchemaError> {
    let mut filtered = items
        .into_iter()
        .map(filter)
        .collect::<Result<Vec<_>, _>>()?;
    let target = if key == "oneOf" {
        "anyOf".to_owned()
    } else {
        key
    };
    if target == "anyOf" {
        match result.get_mut(&target) {
            Some(Json::List(existing)) => {
                existing.append(&mut filtered);
                return Ok(());
            }
            Some(_) => return Err(SchemaError::Invalid("anyOf")),
            None => {}
        }
    }
    result.insert(target, Json::List(filtered));
    Ok(())
}

fn enum_type_conflict(schema: &Object) -> Result<bool, SchemaError> {
    let Some(Json::List(values)) = schema.get("enum") else {
        return Ok(false);
    };
    match schema.get("type") {
        Some(Json::List(_)) => Ok(true),
        Some(Json::Object(_)) => Err(SchemaError::Invalid("type")),
        Some(Json::Text(kind)) => Ok(values.iter().any(|value| !matches_type(value, kind))),
        Some(Json::Null | Json::Bool(_) | Json::Number(_)) | None => Ok(false),
    }
}

fn matches_type(value: &Json, kind: &str) -> bool {
    match kind {
        "null" => matches!(value, Json::Null),
        "boolean" => matches!(value, Json::Bool(_)),
        "integer" => {
            matches!(value, Json::Number(number) if !number.to_string().contains(['.', 'e', 'E']))
        }
        "number" => matches!(value, Json::Number(_)),
        "string" => matches!(value, Json::Text(_)),
        "array" => matches!(value, Json::List(_)),
        "object" => matches!(value, Json::Object(_)),
        _ => true,
    }
}
