use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesTransformError;

pub(super) const STEP_TYPE_ROLES: [(&str, &str); 2] =
    [("user_input", "user"), ("model_output", "assistant")];

pub(super) fn is_history_item(item: &JsonValue) -> bool {
    let JsonValue::Object(fields) = item else {
        return false;
    };
    fields.contains_key("role") || step_role(fields.get("type")).is_some()
}

pub(super) fn responses_role(
    fields: &IndexMap<String, JsonValue>,
) -> Result<String, ResponsesTransformError> {
    if let Some(role) = step_role(fields.get("type")) {
        return Ok(role.to_owned());
    }
    let role = match fields.get("role") {
        None | Some(JsonValue::Null) => "user",
        Some(JsonValue::Text(text)) => text.as_str(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => {
            return Err(ResponsesTransformError::NotText { field: "role" });
        }
    };
    // LiteLLM's `role or "user"` fallback treats a missing or empty role as user
    if role.is_empty() {
        return Ok(String::from("user"));
    }
    if role == "model" {
        return Ok(String::from("assistant"));
    }
    Ok(role.to_owned())
}

fn step_role(kind: Option<&JsonValue>) -> Option<&'static str> {
    let Some(JsonValue::Text(text)) = kind else {
        return None;
    };
    for (step_type, role) in STEP_TYPE_ROLES {
        if step_type == text.as_str() {
            return Some(role);
        }
    }
    None
}
