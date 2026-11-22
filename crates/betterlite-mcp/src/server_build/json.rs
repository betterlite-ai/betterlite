use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;

#[must_use]
pub fn deserialize_json_dict(value: Option<&JsonValue>) -> Option<BTreeMap<String, JsonValue>> {
    match value? {
        JsonValue::Object(entries) => (!entries.is_empty()).then(|| entries.clone()),
        JsonValue::String(text) => object_from_text(text),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => None,
    }
}

#[must_use]
pub fn deserialize_json_list(value: Option<&JsonValue>) -> Option<Vec<JsonValue>> {
    match value? {
        JsonValue::Array(entries) => (!entries.is_empty()).then(|| entries.clone()),
        JsonValue::String(text) => list_from_text(text),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_) => None,
    }
}

fn object_from_text(text: &str) -> Option<BTreeMap<String, JsonValue>> {
    serde_json::from_str(text).ok()
}

fn list_from_text(text: &str) -> Option<Vec<JsonValue>> {
    serde_json::from_str(text).ok()
}

pub fn normalize_cost_info(info: &mut BTreeMap<String, JsonValue>) {
    let Some(JsonValue::Object(cost_info)) = info.get("mcp_server_cost_info").cloned() else {
        return;
    };
    let server_name = info.get("server_name").cloned();
    let mut normalized = cost_info;
    normalize_default_cost(&mut normalized, server_name.as_ref());
    normalize_tool_costs(&mut normalized, server_name.as_ref());
    info.insert(
        "mcp_server_cost_info".to_owned(),
        JsonValue::Object(normalized),
    );
}

fn normalize_default_cost(
    normalized: &mut BTreeMap<String, JsonValue>,
    server_name: Option<&JsonValue>,
) {
    let Some(original) = normalized.get("default_cost_per_query").cloned() else {
        return;
    };
    if matches!(original, JsonValue::Null) {
        return;
    }
    match cost_number(&original) {
        Some(number) => {
            normalized.insert("default_cost_per_query".to_owned(), number);
        }
        None => {
            tracing::warn!(
                "MCP server '{}' has non-numeric default_cost_per_query {}; ignoring it",
                display_text(server_name),
                repr_text(&original)
            );
            normalized.remove("default_cost_per_query");
        }
    }
}

fn normalize_tool_costs(
    normalized: &mut BTreeMap<String, JsonValue>,
    server_name: Option<&JsonValue>,
) {
    let Some(JsonValue::Object(tool_costs)) =
        normalized.get("tool_name_to_cost_per_query").cloned()
    else {
        return;
    };
    let mut parsed = BTreeMap::new();
    for (tool_name, cost) in tool_costs {
        match cost_number(&cost) {
            Some(number) => {
                parsed.insert(tool_name, number);
            }
            None => tracing::warn!(
                "MCP server '{}' has non-numeric cost {} for tool '{}'; ignoring it",
                display_text(server_name),
                repr_text(&cost),
                tool_name
            ),
        }
    }
    normalized.insert(
        "tool_name_to_cost_per_query".to_owned(),
        JsonValue::Object(parsed),
    );
}

fn cost_number(value: &JsonValue) -> Option<JsonValue> {
    let number = match value {
        JsonValue::Number(number) => number.as_f64()?,
        JsonValue::String(text) => text.trim().parse::<f64>().ok()?,
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            return None;
        }
    };
    serde_json::Number::from_f64(number).map(JsonValue::Number)
}

fn display_text(value: Option<&JsonValue>) -> String {
    match value {
        None => "None".to_owned(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(stored) => repr_text(stored),
    }
}

fn repr_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_owned(),
        JsonValue::Bool(true) => "True".to_owned(),
        JsonValue::Bool(false) => "False".to_owned(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => format!("'{text}'"),
        stored @ (JsonValue::Array(_) | JsonValue::Object(_)) => {
            serde_json::to_string(stored).unwrap_or_default()
        }
    }
}
