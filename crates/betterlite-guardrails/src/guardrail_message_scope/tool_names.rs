use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

mod tests;

#[must_use]
pub fn openai_tool_name(tool: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(fields) = tool else {
        return None;
    };
    let name_value = if let Some(GuardrailValue::Object(function)) = fields.get("function") {
        function.get("name")
    } else {
        fields.get("name")
    };
    let Some(GuardrailValue::Text(name)) = name_value else {
        return None;
    };
    Some(name)
}

// A /v1/messages tool can act under its flat name and under function.name, so an allowlist has to see both.
#[must_use]
pub fn anthropic_tool_names(tool: &GuardrailValue) -> Vec<&str> {
    let GuardrailValue::Object(fields) = tool else {
        return Vec::new();
    };
    let is_function_tool =
        matches!(fields.get("type"), Some(GuardrailValue::Text(kind)) if kind == "function");
    let function_name = match fields.get("function") {
        Some(GuardrailValue::Object(function)) if is_function_tool => function.get("name"),
        Some(_) | None => None,
    };
    [fields.get("name"), function_name]
        .into_iter()
        .flatten()
        .filter_map(|candidate| match candidate {
            GuardrailValue::Text(text) if !text.is_empty() => Some(text.as_str()),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_) => None,
        })
        .collect()
}
