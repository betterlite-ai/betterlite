use betterlite_anthropic::capabilities::budget::cap_thinking_to_max_tokens;
use betterlite_prompts::arguments::json::{Json, Object};

use super::capabilities::{ADAPTIVE_THINKING, ThinkingCapabilities};
use super::error::TransformError;
use super::json::from_contract;

pub(crate) struct ReasoningAlias<'request> {
    pub(crate) model: &'request str,
    pub(crate) custom_llm_provider: &'request str,
    pub(crate) max_tokens: i64,
    pub(crate) optional_params: &'request mut Object,
}

pub(crate) async fn translate_reasoning_effort<Capabilities>(
    capabilities: &mut Capabilities,
    mut request: ReasoningAlias<'_>,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let Some(alias) = request.optional_params.shift_remove("reasoning_effort") else {
        return Ok(());
    };
    let Json::Text(effort) = alias else {
        return Ok(());
    };
    let mapped = capabilities
        .map_effort(request.model, &effort, request.custom_llm_provider)
        .await
        .map_err(|error| TransformError::Anthropic {
            message: error.to_string(),
        })?;
    let Some(mapped_thinking) = mapped else {
        request.optional_params.shift_remove("thinking");
        request.optional_params.shift_remove("output_config");
        return Ok(());
    };
    let Some(fitted) = cap_thinking_to_max_tokens(&mapped_thinking, Some(request.max_tokens))
    else {
        // The oracle compares warning text, so this stays LiteLLM's own sentence.
        tracing::warn!(
            "Dropping `thinking` mapped from reasoning_effort={effort} for model={}: max_tokens={} is too small to fit the minimum thinking budget.",
            request.model,
            request.max_tokens
        );
        return Ok(());
    };
    if !request.optional_params.contains_key("thinking") {
        request
            .optional_params
            .insert("thinking".to_owned(), from_contract(&fitted)?);
    }
    apply_output_effort(capabilities, &effort, &mut request).await
}

async fn apply_output_effort<Capabilities>(
    capabilities: &mut Capabilities,
    effort: &str,
    request: &mut ReasoningAlias<'_>,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    if !capabilities
        .supports(
            request.model,
            ADAPTIVE_THINKING,
            request.custom_llm_provider,
        )
        .await
    {
        return Ok(());
    }
    let Some(mapped_effort) = output_config_effort(effort) else {
        return Err(TransformError::Anthropic {
            message: format!(
                "Invalid reasoning_effort: '{effort}'. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'"
            ),
        });
    };
    if let Some(message) = capabilities
        .validate_effort(
            request.model,
            Some(mapped_effort),
            request.custom_llm_provider,
        )
        .await
    {
        return Err(TransformError::Anthropic { message });
    }
    let mut output_config = existing_output_config(request.optional_params);
    output_config
        .entry("effort".to_owned())
        .or_insert_with(|| Json::Text(mapped_effort.to_owned()));
    request
        .optional_params
        .insert("output_config".to_owned(), Json::Object(output_config));
    Ok(())
}

// setdefault reads an existing dict, and anything else starts from an empty one.
fn existing_output_config(params: &Object) -> Object {
    match params.get("output_config") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Object::new(),
    }
}

// "none" already returned through the alias, so every remaining level has an effort string.
fn output_config_effort(effort: &str) -> Option<&'static str> {
    match effort {
        "low" | "minimal" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}
