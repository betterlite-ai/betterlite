use std::collections::BTreeSet;

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
    let object = value.and_then(Value::as_object).map(|value| {
        value
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    });
    object
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
