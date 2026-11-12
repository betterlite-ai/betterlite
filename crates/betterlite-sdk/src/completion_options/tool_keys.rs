use betterlite_models::decisions::Json;

const ANTHROPIC_ONLY_TOOL_KEYS: [&str; 2] = ["input_examples", "eager_input_streaming"];

#[must_use]
pub fn is_claude_tool_target(custom_llm_provider: Option<&str>, model: &str) -> bool {
    let lowered = model.to_lowercase();
    match custom_llm_provider {
        Some("anthropic") => true,
        Some("bedrock") => {
            lowered.contains("claude")
                || (lowered.contains("arn:") && lowered.contains(":bedrock:"))
        }
        Some("azure_ai" | "vertex_ai") => lowered.contains("claude"),
        Some(_) | None => false,
    }
}

#[must_use]
pub fn drop_anthropic_only_tool_keys(tools: Option<&[Json]>) -> Option<Vec<Json>> {
    let mut kept = tools?.to_vec();
    for tool in &mut kept {
        let Json::Object(fields) = tool else {
            continue;
        };
        fields.retain(|key, _value| !ANTHROPIC_ONLY_TOOL_KEYS.contains(&key.as_str()));
        if let Some(Json::Object(function)) = fields.get_mut("function") {
            function.retain(|key, _value| !ANTHROPIC_ONLY_TOOL_KEYS.contains(&key.as_str()));
        }
    }
    Some(kept)
}
