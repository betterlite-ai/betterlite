use std::collections::BTreeMap;

use serde_json::Value;

use crate::types::JsonObject;

pub(super) fn tool_input(item: &JsonObject) -> (String, String, JsonObject, bool) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        "command_execution" => (
            "bash".to_owned(),
            kind.to_owned(),
            BTreeMap::from([(
                "command".to_owned(),
                item.get("command")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        "file_change" => (
            "edit".to_owned(),
            "apply_patch".to_owned(),
            BTreeMap::from([(
                "changes".to_owned(),
                item.get("changes")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            )]),
            true,
        ),
        "web_search" => (
            "web_search".to_owned(),
            kind.to_owned(),
            BTreeMap::from([(
                "query".to_owned(),
                item.get("query")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        _ => {
            let server = item
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let native = item.get("tool").and_then(Value::as_str).unwrap_or_default();
            let input = match item.get("arguments") {
                Some(Value::Object(map)) => map
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                Some(value) => BTreeMap::from([("arguments".to_owned(), value.clone())]),
                None => BTreeMap::new(),
            };
            (
                if server.is_empty() {
                    native.to_owned()
                } else {
                    format!("{server}.{native}")
                },
                native.to_owned(),
                input,
                false,
            )
        }
    }
}
