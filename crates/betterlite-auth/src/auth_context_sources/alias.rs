use betterlite_models::decisions::Json;

// resolve_model_group_alias: both entry shapes, and a malformed entry rewrites nothing.
#[must_use]
pub fn resolve_model_group_alias(model_group_alias: Option<&Json>, model: &str) -> Option<String> {
    let Some(Json::Object(entries)) = model_group_alias else {
        return None;
    };
    let entry = entries.get(model)?;
    let target = match entry {
        Json::Object(item) => item.get("model")?,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => entry,
    };
    match target {
        Json::Text(name) if !name.is_empty() => Some(name.clone()),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
