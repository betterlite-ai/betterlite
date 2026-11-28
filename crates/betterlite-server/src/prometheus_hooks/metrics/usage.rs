use betterlite_logging::prometheus_exporter::{Handle, PrometheusCollectors};
use betterlite_logging::prometheus_usage::metric::UsageMetric;

pub(super) fn usage_handle(collectors: &PrometheusCollectors, metric: UsageMetric) -> Handle {
    match metric {
        UsageMetric::Requests => collectors.litellm_requests_metric.clone(),
        UsageMetric::Spend => collectors.litellm_spend_metric.clone(),
        UsageMetric::ZeroCostRequests => collectors.litellm_zero_cost_requests_total.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::TotalTokens => collectors.litellm_tokens_metric.clone(),
        UsageMetric::InputTokens => collectors.litellm_input_tokens_metric.clone(),
        UsageMetric::OutputTokens => collectors.litellm_output_tokens_metric.clone(),
        UsageMetric::InputCachedTokens => collectors.litellm_input_cached_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::InputCacheCreationTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_input_cache_creation_tokens_metric
                .clone()
        }
        UsageMetric::InputAudioTokens => collectors.litellm_input_audio_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::OutputReasoningTokens => {
            collectors.litellm_output_reasoning_tokens_metric.clone() // ast-grep-ignore: no-long-names
        }
        UsageMetric::OutputAudioTokens => collectors.litellm_output_audio_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::VideoDurationSeconds => {
            collectors.litellm_video_duration_seconds_metric.clone() // ast-grep-ignore: no-long-names
        }
        UsageMetric::ImagesGenerated => collectors.litellm_images_generated_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::CacheHits => collectors.litellm_cache_hits_metric.clone(),
        UsageMetric::CacheMisses => collectors.litellm_cache_misses_metric.clone(),
        UsageMetric::CachedTokens => collectors.litellm_cached_tokens_metric.clone(),
        UsageMetric::ProviderCacheReadTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_provider_cache_read_input_tokens_metric
                .clone()
        }
        UsageMetric::ProviderCacheCreationTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_provider_cache_creation_input_tokens_metric
                .clone()
        }
        UsageMetric::McpToolCalls => collectors.litellm_mcp_tool_calls_total.clone(),
        UsageMetric::McpToolCallSpend => collectors.litellm_mcp_tool_call_spend_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::SpendCaptureRate => collectors.litellm_spend_capture_rate.clone(),
    }
}
