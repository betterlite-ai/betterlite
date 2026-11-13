use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModeError {
    #[error("event_hook.{field} must be a hook name or a list of hook names, got {kind}")]
    NotAMode {
        field: &'static str,
        kind: &'static str,
    },
    #[error("event_hook.tags is required in a mode object")]
    MissingTags,
}

pub fn configured_modes(event_hook: Option<&Json>) -> Result<Option<Vec<String>>, ModeError> {
    let Some(configured) = event_hook else {
        return Ok(None);
    };
    let names = match configured {
        Json::Text(name) => vec![name.clone()],
        Json::List(items) => names_from_list(items)?,
        Json::Object(fields) => names_from_mode(fields)?,
        Json::Null | Json::Bool(_) | Json::Number(_) => return Ok(None),
    };
    Ok(first_occurrences(names))
}

fn names_from_list(items: &[Json]) -> Result<Vec<String>, ModeError> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(name) = item else {
            return Err(ModeError::NotAMode {
                field: "entries",
                kind: kind_of(item),
            });
        };
        names.push(name.clone());
    }
    Ok(names)
}

fn names_from_mode(fields: &Object) -> Result<Vec<String>, ModeError> {
    let mut names = Vec::new();
    if let Some(default) = fields.get("default") {
        push_names(default, "default", &mut names)?;
    }
    let tags = fields.get("tags").ok_or(ModeError::MissingTags)?;
    let Json::Object(tag_values) = tags else {
        return Err(ModeError::NotAMode {
            field: "tags",
            kind: kind_of(tags),
        });
    };
    for value in tag_values.values() {
        push_names(value, "tags", &mut names)?;
    }
    Ok(names)
}

fn push_names(value: &Json, field: &'static str, names: &mut Vec<String>) -> Result<(), ModeError> {
    match value {
        Json::Text(name) => names.push(name.clone()),
        Json::List(items) => {
            for item in items {
                push_names(item, field, names)?;
            }
        }
        Json::Null => {}
        Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            return Err(ModeError::NotAMode {
                field,
                kind: kind_of(value),
            });
        }
    }
    Ok(())
}

fn first_occurrences(names: Vec<String>) -> Option<Vec<String>> {
    let mut unique: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    (!unique.is_empty()).then_some(unique)
}

const fn kind_of(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    }
}

mod tests;
