use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};

const FUNCTION: &str = "function";
const TOOLS: &str = "tools";
const TOOL_CHOICE: &str = "tool_choice";

pub struct DroppedTools {
    pub params: Object,
    pub dropped_types: Vec<String>,
}

pub fn drop_unsupported_tools(optional_params: &Object) -> DroppedTools {
    let mut result = DroppedTools {
        params: optional_params.clone(),
        dropped_types: Vec::new(),
    };
    let Some(Json::List(tools)) = optional_params.get(TOOLS) else {
        return result;
    };
    let (functions, others): (Vec<&Json>, Vec<&Json>) =
        tools.iter().partition(|tool| function_entry(tool).is_ok());
    if others.is_empty() {
        return result;
    }
    let names: BTreeSet<String> = others.into_iter().map(dropped_type_name).collect();
    result.dropped_types = names.into_iter().collect();
    tracing::warn!(
        provider = "deepseek",
        dropped = ?result.dropped_types,
        "DeepSeek chat completions only supports function tools; dropping unsupported tool types"
    );
    result.params.shift_remove(TOOLS);
    if functions.is_empty() {
        result.params.shift_remove(TOOL_CHOICE);
        result.params.shift_remove("parallel_tool_calls");
        return result;
    }
    let survivors: BTreeSet<&str> = functions
        .iter()
        .filter_map(|tool| function_entry(tool).ok().flatten())
        .collect();
    // A string choice, or none at all, survives; a dict must name a function that survived
    if let Some(choice @ Json::Object(_)) = result.params.get(TOOL_CHOICE)
        && !function_entry(choice)
            .ok()
            .flatten()
            .is_some_and(|name| survivors.contains(name))
    {
        result.params.shift_remove(TOOL_CHOICE);
    }
    let kept = functions.into_iter().cloned().collect();
    result.params.insert(TOOLS.to_owned(), Json::List(kept));
    result
}

// Err for a non-function entry; Ok(name) for a function entry, whose name may be missing
fn function_entry(tool: &Json) -> Result<Option<&str>, ()> {
    let fields = tool.as_object().ok_or(())?;
    if fields.get("type").and_then(Json::as_str) != Some(FUNCTION) {
        return Err(());
    }
    let function = fields.get(FUNCTION).and_then(Json::as_object);
    Ok(function
        .and_then(|body| body.get("name"))
        .and_then(Json::as_str))
}

fn dropped_type_name(tool: &Json) -> String {
    match tool.as_object().map(|fields| fields.get("type")) {
        Some(Some(Json::Text(text))) => text.clone(),
        Some(Some(other)) => json_type_name(other).to_owned(),
        Some(None) => "none".to_owned(),
        None => json_type_name(tool).to_owned(),
    }
}

const fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "array",
        Json::Object(_) => "object",
    }
}
