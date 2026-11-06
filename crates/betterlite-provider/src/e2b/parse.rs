use std::collections::BTreeMap;

use betterlite_models::sandbox::{CodeExecutionResult, ExecutionError};
use serde_json::value::RawValue;

#[must_use]
pub fn parse_ndjson(lines: &[String]) -> CodeExecutionResult {
    let mut result = CodeExecutionResult::default();
    let mut count_seen = false;
    for line in lines {
        let Ok(fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(line.trim())
        else {
            continue;
        };
        let Some(kind) = fields
            .get("type")
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        else {
            continue;
        };
        match kind.as_str() {
            "stdout" => result.stdout.push_str(&text(&fields, "text")),
            "stderr" => result.stderr.push_str(&text(&fields, "text")),
            "result" => result.results.push(
                fields
                    .into_iter()
                    .filter(|(key, _)| key != "type")
                    .collect(),
            ),
            "error" if result.error.is_none() => {
                result.error = Some(ExecutionError {
                    name: fields.get("name").cloned(),
                    value: fields.get("value").cloned(),
                    traceback: fields.get("traceback").cloned(),
                });
            }
            "number_of_executions" if !count_seen => {
                count_seen = true;
                result.execution_count = fields
                    .get("execution_count")
                    .and_then(|raw| serde_json::from_str::<i64>(raw.get()).ok());
            }
            _ => {}
        }
    }
    result
}

fn text(fields: &BTreeMap<String, Box<RawValue>>, key: &str) -> String {
    fields
        .get(key)
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .unwrap_or_default()
}
