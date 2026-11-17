use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{EffortThresholds, OptionalParams};
use betterlite_anthropic::params::ParamsRequest;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::map_openai_params as map_parent_params;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use super::PROVIDER;

// LiteLLM maps a schema-backed response_format through this model so the tool shape is chosen.
pub const RESPONSE_FORMAT_FALLBACK_MODEL: &str = "claude-3-sonnet-20240229";

// The response_format branch reads the model's native structured-output capability, maps through the
// fallback model when the model has none, then translates legacy thinking for the model actually asked for.
pub async fn map_openai_params<Source, Transport>(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let fallback = if non_default_params.contains_key("response_format") {
        !supports_model_capability(
            request.model,
            "supports_native_structured_output",
            PROVIDER,
            environment,
            request.catalogs,
        )
        .await
    } else {
        false
    };
    let parent = ParamsRequest {
        model: if fallback {
            RESPONSE_FORMAT_FALLBACK_MODEL
        } else {
            request.model
        },
        provider: PROVIDER,
        drop: request.drop,
        budgets: request.budgets,
        default_max_tokens: request.default_max_tokens,
        catalogs: request.catalogs,
    };
    map_parent_params(&parent, non_default_params, optional_params, environment).await?;
    translate_thinking(request, optional_params, environment).await
}

// LiteLLM restores the caller's model before this runs, so the adaptive conversion sees the real one.
async fn translate_thinking<Source, Transport>(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let mut params = OptionalParams {
        reasoning_effort: optional_params.get("reasoning_effort").cloned(),
        thinking: optional_params.get("thinking").cloned(),
        output_config: optional_params.get("output_config").cloned(),
    };
    let before_thinking = params.thinking.clone();
    let before_output = params.output_config.clone();
    let thinking_request = ThinkingRequest {
        model: request.model,
        custom_llm_provider: PROVIDER,
        thresholds: EffortThresholds {
            medium: request.budgets.medium,
            high: request.budgets.high,
            xhigh: request.budgets.xhigh,
        },
    };
    translate_legacy_thinking(
        &thinking_request,
        &mut params,
        environment,
        request.catalogs,
    )
    .await?;
    if params.thinking != before_thinking {
        match params.thinking {
            Some(thinking) => optional_params.insert("thinking".to_owned(), thinking),
            None => optional_params.shift_remove("thinking"),
        };
    }
    if params.output_config != before_output
        && let Some(output_config) = params.output_config
    {
        optional_params.insert("output_config".to_owned(), output_config);
    }
    Ok(())
}
