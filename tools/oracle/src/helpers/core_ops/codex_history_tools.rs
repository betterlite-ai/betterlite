pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match operation {
            "normalize" => {
                let value = input.get("input").unwrap_or(&serde_json::Value::Null);
                let (normalized, rewritten_types) =
                    betterlite_core::api::codex_history_tools::normalize_codex_input_items(value);
                Ok(serde_json::json!({"input": normalized, "rewritten_types": rewritten_types}))
            }
            "drop_unsupported_tools" => {
                let tools = input
                    .get("tools")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let supported_types = input
                    .get("supported_types")
                    .and_then(serde_json::Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let (kept, dropped_types) =
                    betterlite_core::api::codex_history_tools::drop_unsupported_tools(
                        &tools,
                        &supported_types,
                    );
                Ok(serde_json::json!({"tools": kept, "dropped_types": dropped_types}))
            }
            "hoist_additional_tools" => {
                let value = input.get("input").unwrap_or(&serde_json::Value::Null);
                let tools = input
                    .get("tools")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::as_slice);
                let result =
                    betterlite_core::api::codex_history_tools::hoist_additional_tools(value, tools);
                Ok(
                    serde_json::json!({"input": result.input, "tools": result.tools, "hoisted": result.hoisted, "debug": result.debug}),
                )
            }
            _ => Self::not_built("codex_history_tools"),
        }
    }
}
