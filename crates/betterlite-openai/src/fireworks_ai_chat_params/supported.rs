use betterlite_models::fireworks_catalog_metadata::{Capability, Catalog, capability_exact};

use super::FireworksCapabilities;

const BASE_SUPPORTED_PARAMS: [&str; 37] = [
    "stream",
    "max_completion_tokens",
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "frequency_penalty",
    "presence_penalty",
    "n",
    "stop",
    "response_format",
    "user",
    "logprobs",
    "prompt_truncate_len",
    "context_length_exceeded_behavior",
    "seed",
    "top_logprobs",
    "min_p",
    "typical_p",
    "repetition_penalty",
    "mirostat_target",
    "mirostat_lr",
    "logit_bias",
    "echo",
    "echo_last",
    "ignore_eos",
    "prompt_cache_key",
    "prompt_cache_isolation_key",
    "raw_output",
    "perf_metrics_in_response",
    "return_token_ids",
    "safe_tokenization",
    "service_tier",
    "speculation",
    "prediction",
    "stream_options",
    "sampling_mask",
];

// LiteLLM's get_supported_openai_params: the fixed list, then the tool and reasoning gates.
#[must_use]
pub async fn supported_openai_params<P>(
    capabilities: &mut P,
    catalog: &Catalog,
    model: &str,
) -> Vec<&'static str>
where
    P: FireworksCapabilities,
{
    let mut params = BASE_SUPPORTED_PARAMS.to_vec();
    if capabilities.supports_function_calling(model).await {
        params.push("tools");
        params.push("parallel_tool_calls");
    } else {
        tracing::debug!(
            model,
            "fireworks_ai model is marked as not supporting function calling; tools and parallel_tool_calls are dropped from the request"
        );
    }
    if capability_exact(catalog, model, Capability::ToolChoice) == Some(true)
        || capabilities.supports_tool_choice(model).await
    {
        params.push("tool_choice");
    }
    if capability_exact(catalog, model, Capability::Reasoning) == Some(true)
        || capabilities.supports_reasoning(model).await
    {
        params.push("reasoning_effort");
        params.push("reasoning_history");
        params.push("thinking");
    }
    params
}
