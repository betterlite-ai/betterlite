use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::model_capability;
use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest, NamedCapability,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::converse_options::family::{
    is_deepseek_model, is_nova_2_model, is_openai_gpt_reasoning_model,
    model_rejects_reasoning_effort_param,
};
use crate::model_routes::{ParamMap, ParamValue};

const CONVERSE_PROVIDER: &str = "bedrock_converse";
const SAMPLING_CAPABILITY: &str = "supports_sampling_params";
const THINKING_MODEL_MARKERS: [&str; 3] = ["claude-3-7", "claude-sonnet-4", "claude-opus-4"];
const TOOL_BASE_PREFIXES: [&str; 8] = [
    "anthropic",
    "mistral",
    "cohere",
    "meta.llama3-1",
    "meta.llama3-2",
    "meta.llama3-3",
    "meta.llama4",
    "amazon.nova",
];
const TOOL_SEARCH_PARAMS: [&str; 5] = [
    "tools",
    "tool_choice",
    "thinking",
    "reasoning_effort",
    "output_config",
];

// LiteLLM probes the raw cost map per candidate; the regionless base is what it prefixes.
#[must_use]
pub fn supports_sampling_params(model: &str, catalogs: &CatalogSources<'_>) -> bool {
    let base_model = get_bedrock_base_model(model);
    if base_model.starts_with("anthropic") {
        return true;
    }
    let candidates = [
        model.to_owned(),
        format!("global.{base_model}"),
        format!("us.{base_model}"),
        format!("eu.{base_model}"),
    ];
    for candidate in candidates {
        if let Some(flag) = model_capability(
            catalogs.patterns,
            catalogs.runtime,
            catalogs.packaged,
            &candidate,
            SAMPLING_CAPABILITY,
        ) {
            return flag;
        }
    }
    true
}

// No DeepSeek accepts the Anthropic-shaped thinking field, even when its supports_reasoning flag is true.
#[must_use]
pub async fn model_accepts_anthropic_thinking_param<Source, Transport>(
    model: &str,
    base_model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if is_deepseek_model(model, base_model) {
        return false;
    }
    if THINKING_MODEL_MARKERS
        .iter()
        .any(|marker| model.contains(marker))
    {
        return true;
    }
    if environment
        .supports_factory(
            &CapabilityRequest {
                model,
                custom_llm_provider: Some(CONVERSE_PROVIDER),
            },
            NamedCapability::Reasoning.key(),
        )
        .await
    {
        return true;
    }
    environment
        .supports_factory(
            &CapabilityRequest {
                model: base_model,
                custom_llm_provider: Some(CONVERSE_PROVIDER),
            },
            NamedCapability::Reasoning.key(),
        )
        .await
}

#[must_use]
pub fn auto_tool_choice() -> ParamMap {
    let mut block = ParamMap::new();
    block.insert("auto".to_owned(), ParamValue::Mapping(ParamMap::new()));
    block
}

#[must_use]
pub async fn supported_openai_params<Source, Transport>(
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Vec<String>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let mut params = base_params(model, catalogs);
    if model.contains("arn") {
        params.extend(TOOL_SEARCH_PARAMS.map(str::to_owned));
        if model.to_lowercase().contains("nova") {
            params.push("web_search_options".to_owned());
        }
        return params;
    }
    let base_model = get_bedrock_base_model(model);
    push_tool_params(&mut params, model, &base_model, environment).await;
    push_reasoning_param(&mut params, model, &base_model, environment).await;
    if base_model.starts_with("anthropic") {
        params.push("context_management".to_owned());
    }
    params
}

fn base_params(model: &str, catalogs: &CatalogSources<'_>) -> Vec<String> {
    let leading = [
        "max_tokens",
        "max_completion_tokens",
        "stream",
        "stream_options",
        "stop",
    ];
    let mut params: Vec<String> = leading.map(str::to_owned).into_iter().collect();
    if supports_sampling_params(model, catalogs) {
        params.push("temperature".to_owned());
        params.push("top_p".to_owned());
    }
    params.extend(
        [
            "extra_headers",
            "response_format",
            "requestMetadata",
            "service_tier",
            "parallel_tool_calls",
        ]
        .map(str::to_owned),
    );
    params
}

async fn push_tool_params<Source, Transport>(
    params: &mut Vec<String>,
    model: &str,
    base_model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some(CONVERSE_PROVIDER),
    };
    let prefixed = TOOL_BASE_PREFIXES
        .iter()
        .any(|prefix| base_model.starts_with(prefix));
    let tool_choice = environment
        .supports_factory(&request, NamedCapability::ToolChoice.key())
        .await
        || environment
            .supports_factory(
                &CapabilityRequest {
                    model: base_model,
                    custom_llm_provider: Some(CONVERSE_PROVIDER),
                },
                NamedCapability::ToolChoice.key(),
            )
            .await;
    if prefixed
        || environment
            .supports_factory(&request, NamedCapability::FunctionCalling.key())
            .await
    {
        params.push("tools".to_owned());
    }
    if base_model.starts_with("amazon.nova") {
        params.push("web_search_options".to_owned());
    }
    if tool_choice {
        params.push("tool_choice".to_owned());
    }
}

enum ReasoningShape {
    Effort,
    Silent,
    Anthropic,
}

fn reasoning_shape(model: &str, base_model: &str) -> ReasoningShape {
    if model.contains("gpt-oss")
        || is_openai_gpt_reasoning_model(model)
        || is_openai_gpt_reasoning_model(base_model)
    {
        return ReasoningShape::Effort;
    }
    if is_deepseek_model(model, base_model) {
        return if model_rejects_reasoning_effort_param(model, base_model) {
            ReasoningShape::Silent
        } else {
            ReasoningShape::Effort
        };
    }
    if is_nova_2_model(model) {
        return ReasoningShape::Effort;
    }
    ReasoningShape::Anthropic
}

async fn push_reasoning_param<Source, Transport>(
    params: &mut Vec<String>,
    model: &str,
    base_model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    match reasoning_shape(model, base_model) {
        ReasoningShape::Effort => params.push("reasoning_effort".to_owned()),
        ReasoningShape::Silent => {}
        ReasoningShape::Anthropic => {
            if model_accepts_anthropic_thinking_param(model, base_model, environment).await {
                params.extend(["thinking", "reasoning_effort", "output_config"].map(str::to_owned));
            }
        }
    }
}
