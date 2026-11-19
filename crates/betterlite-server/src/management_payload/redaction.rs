use betterlite_db::managed_resource_rows::mcp::{McpEnvVar, McpServerTable};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

// The value field of every projected env var, whatever the entry carried
const BLANKED_VALUE: &str = "";

// LiteLLM's _object_mapping: only a JSON object is a mapping, nothing else is
#[must_use]
pub const fn object_mapping(value: &Json) -> Option<&JsonObject> {
    match value {
        Json::Object(mapping) => Some(mapping),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// LiteLLM's _object_list: only a JSON list is a list, nothing else is
#[must_use]
pub const fn object_list(value: &Json) -> Option<&[Json]> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

// LiteLLM's _redacted_env_var: name, scope and description as carried, everything else dropped
#[must_use]
pub fn redacted_env_var(entry: &Json) -> Json {
    let Some(fields) = object_mapping(entry) else {
        return env_var_projection(None, None, None);
    };
    env_var_projection(
        fields.get("name"),
        fields.get("scope"),
        fields.get("description"),
    )
}

// The four keys of LiteLLM's _redacted_env_var literal, with a null for each absent attribute
fn env_var_projection(
    name: Option<&Json>,
    scope: Option<&Json>,
    description: Option<&Json>,
) -> Json {
    Json::Object(IndexMap::from([
        ("name".to_owned(), name.map_or(Json::Null, Clone::clone)),
        ("scope".to_owned(), scope.map_or(Json::Null, Clone::clone)),
        (
            "description".to_owned(),
            description.map_or(Json::Null, Clone::clone),
        ),
        ("value".to_owned(), Json::Text(BLANKED_VALUE.to_owned())),
    ]))
}

// LiteLLM's _redact_record_env_vars copies the dict record and blanks env values, never the caller's
#[must_use]
pub fn redact_record_env_vars(record: &Json) -> Json {
    let Some(mapping) = object_mapping(record) else {
        return record.clone();
    };
    let Some(entries) = mapping.get("env_vars").and_then(object_list) else {
        return record.clone();
    };
    let mut redacted = mapping.clone();
    redacted.insert(
        "env_vars".to_owned(),
        Json::List(entries.iter().map(redacted_env_var).collect()),
    );
    Json::Object(redacted)
}

// LiteLLM's _redact_record_env_vars on a BaseModel record: model_copy(update={"env_vars": redacted})
#[must_use]
pub fn redact_model_env_vars(record: &McpServerTable) -> McpServerTable {
    let Some(entries) = record.env_vars.as_ref() else {
        return record.clone();
    };
    let mut redacted = record.clone();
    redacted.env_vars = Some(entries.iter().map(redacted_model_env_var).collect());
    redacted
}

fn redacted_model_env_var(entry: &McpEnvVar) -> McpEnvVar {
    McpEnvVar {
        name: entry.name.clone(),
        value: BLANKED_VALUE.to_owned(),
        scope: entry.scope,
        description: entry.description.clone(),
    }
}

// LiteLLM's _redact_env_var_values reaches the top-level env_vars and one items level, not deeper
pub fn redact_env_var_values(response: &mut JsonObject) {
    if let Some(entries) = response.get("env_vars").and_then(object_list) {
        let redacted: Vec<Json> = entries.iter().map(redacted_env_var).collect();
        response.insert("env_vars".to_owned(), Json::List(redacted));
    }
    if let Some(items) = response.get("items").and_then(object_list) {
        let redacted: Vec<Json> = items.iter().map(redact_record_env_vars).collect();
        response.insert("items".to_owned(), Json::List(redacted));
    }
}
