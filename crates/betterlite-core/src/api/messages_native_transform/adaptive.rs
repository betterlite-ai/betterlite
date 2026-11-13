use betterlite_anthropic::capabilities::budget::cap_thinking_to_max_tokens;
use betterlite_prompts::arguments::json::{Json, Object};

use super::capabilities::{ADAPTIVE_THINKING, REASONING, ThinkingCapabilities};
use super::error::TransformError;
use super::json::from_contract;

pub(crate) struct AdaptiveEffort<'request> {
    pub(crate) model: &'request str,
    pub(crate) custom_llm_provider: &'request str,
    pub(crate) max_tokens: i64,
    pub(crate) optional_params: &'request mut Object,
}

pub(crate) async fn translate_adaptive_effort<Capabilities>(
    capabilities: &mut Capabilities,
    mut request: AdaptiveEffort<'_>,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    if capabilities
        .supports(
            request.model,
            ADAPTIVE_THINKING,
            request.custom_llm_provider,
        )
        .await
    {
        return Ok(());
    }
    let output_config = request.optional_params.get("output_config").cloned();
    let effort = output_config.as_ref().and_then(effort_text);
    let adaptive_thinking = is_adaptive(request.optional_params.get("thinking"));
    if effort.is_none() && !adaptive_thinking {
        return Ok(());
    }
    if keeps_native_effort(capabilities, &request, effort.as_deref(), adaptive_thinking).await {
        if adaptive_thinking {
            request.optional_params.shift_remove("thinking");
        }
        return Ok(());
    }
    apply_legacy_effort(capabilities, &mut request, effort.as_deref()).await?;
    drop_consumed_effort(request.optional_params, output_config);
    Ok(())
}

// Opus 4.5 keeps a native effort and only loses the adaptive thinking block it cannot accept.
async fn keeps_native_effort<Capabilities>(
    capabilities: &mut Capabilities,
    request: &AdaptiveEffort<'_>,
    effort: Option<&str>,
    adaptive_thinking: bool,
) -> bool
where
    Capabilities: ThinkingCapabilities + Send,
{
    if !capabilities
        .supports_effort_param(request.model, request.custom_llm_provider)
        .await
    {
        return false;
    }
    !adaptive_thinking
        || capabilities
            .validate_effort(request.model, effort, request.custom_llm_provider)
            .await
            .is_none()
}

async fn apply_legacy_effort<Capabilities>(
    capabilities: &mut Capabilities,
    request: &mut AdaptiveEffort<'_>,
    effort: Option<&str>,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let supports_thinking = capabilities
        .supports(request.model, REASONING, request.custom_llm_provider)
        .await;
    let legacy = if supports_thinking {
        capabilities
            .map_effort(
                request.model,
                effort.unwrap_or("medium"),
                request.custom_llm_provider,
            )
            .await
            .map_err(|error| TransformError::Anthropic {
                message: error.to_string(),
            })?
    } else {
        None
    };
    let capped =
        legacy.and_then(|thinking| cap_thinking_to_max_tokens(&thinking, Some(request.max_tokens)));
    let Some(thinking) = capped else {
        // The oracle compares warning text, so this stays LiteLLM's own sentence.
        tracing::warn!(
            "Dropping adaptive `thinking`/`output_config.effort` for model={}: the model does not support extended thinking, or max_tokens is too small to fit the minimum thinking budget.",
            request.model
        );
        request.optional_params.shift_remove("thinking");
        return Ok(());
    };
    request
        .optional_params
        .insert("thinking".to_owned(), from_contract(&thinking)?);
    Ok(())
}

// Only the consumed effort key leaves output_config; any residual stays for a provider subclass.
fn drop_consumed_effort(params: &mut Object, output_config: Option<Json>) {
    let Some(Json::Object(fields)) = output_config else {
        return;
    };
    if !fields.contains_key("effort") {
        return;
    }
    let residual: Object = fields
        .into_iter()
        .filter(|(key, _value)| key != "effort")
        .collect();
    if residual.is_empty() {
        params.shift_remove("output_config");
    } else {
        params.insert("output_config".to_owned(), Json::Object(residual));
    }
}

fn effort_text(output_config: &Json) -> Option<String> {
    let Json::Object(fields) = output_config else {
        return None;
    };
    fields
        .get("effort")
        .and_then(Json::as_str)
        .map(str::to_owned)
}

fn is_adaptive(thinking: Option<&Json>) -> bool {
    matches!(
        thinking,
        Some(Json::Object(fields))
            if fields.get("type").and_then(Json::as_str) == Some("adaptive")
    )
}
