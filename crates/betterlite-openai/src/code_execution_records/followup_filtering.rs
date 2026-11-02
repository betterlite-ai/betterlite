use serde_json::{Map, Value};

use super::tool_shapes::{function_tool, tool_choice_detection};
use super::{CallDialect, OTHER_INTERCEPTION_PREFIXES};

#[must_use]
pub fn agentic_loop_kwargs(kwargs: &Map<String, Value>) -> Map<String, Value> {
    kwargs
        .iter()
        .filter(|(key, _)| {
            key.as_str() != "litellm_logging_obj"
                && key.as_str() != "acompletion"
                && !OTHER_INTERCEPTION_PREFIXES
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[must_use]
pub fn tools(requested_tools: &Value, dialect: CallDialect) -> Option<Vec<Value>> {
    let items = requested_tools.as_array()?;
    let replacement = serde_json::to_value(function_tool(dialect)).unwrap_or(Value::Null);
    Some(
        items
            .iter()
            .map(|tool| {
                let targets_code_interpreter = tool
                    .as_object()
                    .and_then(|object| object.get("type"))
                    .and_then(Value::as_str)
                    == Some("code_interpreter");
                if targets_code_interpreter {
                    replacement.clone()
                } else {
                    tool.clone()
                }
            })
            .collect(),
    )
}

pub fn optional_params(optional_params: &Map<String, Value>) -> Map<String, Value> {
    let drop_tool_choice = optional_params
        .get("tool_choice")
        .is_some_and(tool_choice_detection);
    let mut filtered = optional_params.clone();
    filtered.retain(|key, _| {
        key.as_str() != "tools" && !(key.as_str() == "tool_choice" && drop_tool_choice)
    });
    filtered
}

include!("followup_filtering/tests/cases.rs");
