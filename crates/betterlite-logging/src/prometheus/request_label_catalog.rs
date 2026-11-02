mod deployment;
mod fallbacks;
mod guardrails;
mod latency;
mod requests;
mod spend;
mod tokens;

mod tests;

const METRIC_LABELS: &[(&str, &[&str])] = &[
    ("litellm_llm_api_latency_metric", latency::API_LATENCY),
    (
        "litellm_llm_api_time_to_first_token_metric",
        latency::TIME_TO_FIRST_TOKEN,
    ),
    (
        "litellm_request_total_latency_metric",
        latency::REQUEST_TOTAL_LATENCY,
    ),
    (
        "litellm_request_queue_time_seconds",
        latency::REQUEST_QUEUE_TIME,
    ),
    (
        "litellm_deployment_latency_per_output_token",
        latency::PER_OUTPUT_TOKEN,
    ),
    ("litellm_overhead_latency_metric", latency::OVERHEAD),
    (
        "litellm_overhead_with_guardrails_latency_metric",
        latency::OVERHEAD_WITH_GUARDRAILS,
    ),
    (
        "litellm_guardrail_latency_seconds",
        guardrails::LATENCY_SECONDS,
    ),
    ("litellm_guardrail_errors_total", guardrails::ERRORS_TOTAL),
    (
        "litellm_guardrail_requests_total",
        guardrails::REQUESTS_TOTAL,
    ),
    ("litellm_proxy_total_requests_metric", requests::PROXY_TOTAL),
    (
        "litellm_proxy_failed_requests_metric",
        requests::PROXY_FAILED,
    ),
    (
        "litellm_remaining_requests_metric",
        requests::REMAINING_REQUESTS,
    ),
    (
        "litellm_remaining_tokens_metric",
        requests::REMAINING_TOKENS,
    ),
    ("litellm_requests_metric", requests::REQUESTS),
    ("litellm_spend_metric", spend::SPEND),
    (
        "litellm_zero_cost_requests_total",
        spend::ZERO_COST_REQUESTS_TOTAL,
    ),
    ("litellm_spend_capture_rate", spend::CAPTURE_RATE),
    ("litellm_input_tokens_metric", tokens::INPUT),
    ("litellm_input_cached_tokens_metric", tokens::INPUT),
    ("litellm_input_cache_creation_tokens_metric", tokens::INPUT),
    ("litellm_input_audio_tokens_metric", tokens::INPUT),
    ("litellm_total_tokens_metric", tokens::TOTAL),
    ("litellm_output_tokens_metric", tokens::OUTPUT),
    ("litellm_output_reasoning_tokens_metric", tokens::OUTPUT),
    ("litellm_output_audio_tokens_metric", tokens::OUTPUT),
    ("litellm_video_duration_seconds_metric", tokens::OUTPUT),
    ("litellm_images_generated_metric", tokens::OUTPUT),
    ("litellm_deployment_state", deployment::STATE),
    ("litellm_deployment_tpm_limit", deployment::TPM_LIMIT),
    ("litellm_deployment_rpm_limit", deployment::TPM_LIMIT),
    ("litellm_deployment_cooled_down", deployment::COOLED_DOWN),
    (
        "litellm_deployment_successful_fallbacks",
        fallbacks::SUCCESSFUL,
    ),
    ("litellm_deployment_failed_fallbacks", fallbacks::SUCCESSFUL),
];

#[must_use]
pub fn labels_for_metric(metric: &str) -> Option<&'static [&'static str]> {
    METRIC_LABELS
        .iter()
        .find(|(name, _)| *name == metric)
        .map(|(_, labels)| *labels)
}
