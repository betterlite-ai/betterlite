pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match operation {
            "normalize" => Ok(normalize(input)),
            "drop_unsupported_tools" => Ok(drop_unsupported(input)),
            "hoist_additional_tools" => Ok(hoist(input)),
            _ => Self::not_built("codex_history_tools"),
        }
    }
}

fn normalize(input: &serde_json::Value) -> serde_json::Value {
    let value = input.get("input").unwrap_or(&serde_json::Value::Null);
    let (normalized, rewritten_types) =
        betterlite_core::api::codex_history_tools::normalize_codex_input_items(value);
    serde_json::json!({"input": normalized, "rewritten_types": rewritten_types})
}

fn drop_unsupported(input: &serde_json::Value) -> serde_json::Value {
    let tools = values(input, "tools");
    let supported_types = supported_types(input);
    let (kept, dropped_types) =
        betterlite_core::api::codex_history_tools::drop_unsupported_tools(&tools, &supported_types);
    serde_json::json!({"tools": kept, "dropped_types": dropped_types})
}

fn supported_types(input: &serde_json::Value) -> std::collections::BTreeSet<String> {
    let Some(values) = input
        .get("supported_types")
        .and_then(serde_json::Value::as_array)
    else {
        return std::collections::BTreeSet::new();
    };
    values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn hoist(input: &serde_json::Value) -> serde_json::Value {
    let value = input.get("input").unwrap_or(&serde_json::Value::Null);
    let tools = values(input, "tools");
    let result = betterlite_core::api::codex_history_tools::hoist_additional_tools(
        value,
        Some(tools.as_slice()),
    );
    serde_json::json!({"input": result.input, "tools": result.tools, "hoisted": result.hoisted, "debug": result.debug})
}

fn values(input: &serde_json::Value, field: &str) -> Vec<serde_json::Value> {
    let Some(values) = input.get(field).and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    values.clone()
}
