use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::types::JsonObject;

#[derive(Clone, Debug, Default)]
pub struct CodexStreamState {
    pub thread_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub failed: bool,
    pub(super) started: BTreeSet<String>,
}

pub(super) fn object(value: Option<&Value>) -> Option<JsonObject> {
    value.and_then(Value::as_object).map(|value| {
        value
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    })
}

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
                Some(Value::Object(map)) => {
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                }
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

pub(super) fn tool_output(item: &JsonObject) -> (String, bool) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let status = item.get("status").and_then(Value::as_str);
    match kind {
        "command_execution" => (
            item.get("aggregated_output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            status == Some("failed")
                || item
                    .get("exit_code")
                    .is_some_and(|v| v.as_i64().is_some_and(|code| code != 0)),
        ),
        "file_change" => (
            item.get("changes")
                .and_then(Value::as_array)
                .map(|changes| {
                    changes
                        .iter()
                        .map(|change| {
                            format!(
                                "{} {}",
                                change
                                    .get("kind")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                change
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            )
                            .trim()
                            .to_owned()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            status == Some("failed"),
        ),
        "web_search" => (String::new(), status == Some("failed")),
        _ => match item.get("error") {
            Some(error) => (
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| error.to_string(), str::to_owned),
                true,
            ),
            None => (
                item.get("result").map_or_else(String::new, |value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                }),
                status == Some("failed"),
            ),
        },
    }
}
