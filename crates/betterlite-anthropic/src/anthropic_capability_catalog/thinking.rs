use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;
use num_traits::cast::cast;

use super::capability::supports_model_capability;
use super::{CatalogSources, EffortThresholds, OptionalParams};

pub const DROP_DISABLED_THINKING_WARNING: &str = "Dropping `thinking={'type': 'disabled'}` for model=%s: thinking is always on for this model and cannot be disabled (the alternative is a provider 400). The model will still think adaptively, its response can contain thinking blocks, and those thinking tokens are billed as output tokens.";

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ThinkingError {
    #[error("thinking budget_tokens is not an integer")]
    BudgetTokens,
}

pub struct ThinkingRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub thresholds: EffortThresholds,
}

pub struct EffortRequest<'request> {
    pub model: &'request str,
    pub budget_tokens: i64,
    pub custom_llm_provider: &'request str,
    pub thresholds: EffortThresholds,
}

pub async fn maybe_drop_disabled_thinking<Source, Transport>(
    model: &str,
    params: &mut OptionalParams,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(Json::Object(thinking)) = params.thinking.as_ref() else {
        return false;
    };
    if !matches!(thinking.get("type"), Some(Json::Text(kind)) if kind == "disabled") {
        return false;
    }
    if !supports_model_capability(
        model,
        "thinking_always_on",
        custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        return false;
    }
    tracing::warn!(
        "{}",
        DROP_DISABLED_THINKING_WARNING.replacen("%s", model, 1)
    );
    params.thinking = None;
    true
}

pub async fn translate_legacy_thinking<Source, Transport>(
    request: &ThinkingRequest<'_>,
    params: &mut OptionalParams,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<(), ThinkingError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if !supports_model_capability(
        request.model,
        "supports_adaptive_thinking",
        request.custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        return Ok(());
    }
    if supports_model_capability(
        request.model,
        "supports_legacy_thinking",
        request.custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        return Ok(());
    }
    let Some(Json::Object(thinking)) = params.thinking.as_ref() else {
        return Ok(());
    };
    if !matches!(thinking.get("type"), Some(Json::Text(kind)) if kind == "enabled") {
        return Ok(());
    }
    let effort_request = EffortRequest {
        model: request.model,
        budget_tokens: budget_tokens(thinking.get("budget_tokens"))?,
        custom_llm_provider: request.custom_llm_provider,
        thresholds: request.thresholds,
    };
    let effort = legacy_budget_to_effort(&effort_request, environment, catalogs).await;
    params.output_config = Some(overlay_output_config(effort, params.output_config.as_ref()));
    params.thinking = Some(adaptive_thinking(thinking.get("display")));
    Ok(())
}

// A summarized or omitted display carries over; any other display value is dropped.
fn adaptive_thinking(requested: Option<&Json>) -> Json {
    let mut adaptive = IndexMap::new();
    adaptive.insert("type".to_owned(), Json::Text("adaptive".to_owned()));
    if let Some(Json::Text(display)) = requested
        && (display == "summarized" || display == "omitted")
    {
        adaptive.insert("display".to_owned(), Json::Text(display.clone()));
    }
    Json::Object(adaptive)
}

// The caller's output_config overlays the inferred effort, so even effort=None wins.
fn overlay_output_config(effort: &str, existing: Option<&Json>) -> Json {
    let mut config = IndexMap::new();
    config.insert("effort".to_owned(), Json::Text(effort.to_owned()));
    if let Some(Json::Object(fields)) = existing {
        for (key, value) in fields {
            config.insert(key.clone(), value.clone());
        }
    }
    Json::Object(config)
}

#[must_use]
pub async fn legacy_budget_to_effort<Source, Transport>(
    request: &EffortRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> &'static str
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let xhigh_supported = request.budget_tokens >= request.thresholds.xhigh
        && supports_model_capability(
            request.model,
            "supports_xhigh_reasoning_effort",
            request.custom_llm_provider,
            environment,
            catalogs,
        )
        .await;
    if xhigh_supported {
        return "xhigh";
    }
    if request.budget_tokens >= request.thresholds.high {
        return "high";
    }
    if request.budget_tokens >= request.thresholds.medium {
        return "medium";
    }
    "low"
}

#[must_use]
pub async fn is_effort_used<Source, Transport>(
    optional_params: Option<&OptionalParams>,
    model: &str,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(params) = optional_params else {
        return false;
    };
    let adaptive = !model.is_empty()
        && supports_model_capability(
            model,
            "supports_adaptive_thinking",
            custom_llm_provider,
            environment,
            catalogs,
        )
        .await;
    if adaptive {
        return false;
    }
    let lowered = model.to_lowercase();
    let opus = lowered.contains("opus-4-5") || lowered.contains("opus_4_5");
    let effort_text = |value: &Json| matches!(value, Json::Text(text) if !text.is_empty());
    if opus && params.reasoning_effort.as_ref().is_some_and(effort_text) {
        return true;
    }
    if let Some(Json::Object(fields)) = params.output_config.as_ref()
        && fields.get("effort").is_some_and(effort_text)
    {
        return true;
    }
    false
}

// Python's int(thinking.get("budget_tokens") or 0): absent and null read as 0, floats truncate.
fn budget_tokens(value: Option<&Json>) -> Result<i64, ThinkingError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Json::Number(number)) => number
            .as_i64()
            .or_else(|| cast::<f64, i64>(number.as_f64()?))
            .ok_or(ThinkingError::BudgetTokens),
        Some(Json::Text(text)) => text
            .trim()
            .parse::<i64>()
            .map_err(|_error| ThinkingError::BudgetTokens),
        Some(Json::List(_) | Json::Object(_)) => Err(ThinkingError::BudgetTokens),
    }
}
