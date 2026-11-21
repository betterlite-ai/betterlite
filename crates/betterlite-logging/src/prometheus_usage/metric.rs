use betterlite_models::zero_cost::ZERO_COST_COUNTER_NAME;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UsageMetric {
    Requests,
    Spend,
    ZeroCostRequests,
    TotalTokens,
    InputTokens,
    OutputTokens,
    InputCachedTokens,
    InputCacheCreationTokens,
    InputAudioTokens,
    OutputReasoningTokens,
    OutputAudioTokens,
    VideoDurationSeconds,
    ImagesGenerated,
    CacheHits,
    CacheMisses,
    CachedTokens,
    ProviderCacheReadTokens,
    ProviderCacheCreationTokens,
    McpToolCalls,
    McpToolCallSpend,
    SpendCaptureRate,
}

impl UsageMetric {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Requests => "litellm_requests_metric",
            Self::Spend => "litellm_spend_metric",
            Self::ZeroCostRequests => ZERO_COST_COUNTER_NAME,
            Self::TotalTokens => "litellm_total_tokens_metric",
            Self::InputTokens => "litellm_input_tokens_metric",
            Self::OutputTokens => "litellm_output_tokens_metric",
            Self::InputCachedTokens => "litellm_input_cached_tokens_metric",
            Self::InputCacheCreationTokens => "litellm_input_cache_creation_tokens_metric",
            Self::InputAudioTokens => "litellm_input_audio_tokens_metric",
            Self::OutputReasoningTokens => "litellm_output_reasoning_tokens_metric",
            Self::OutputAudioTokens => "litellm_output_audio_tokens_metric",
            Self::VideoDurationSeconds => "litellm_video_duration_seconds_metric",
            Self::ImagesGenerated => "litellm_images_generated_metric",
            Self::CacheHits => "litellm_cache_hits_metric",
            Self::CacheMisses => "litellm_cache_misses_metric",
            Self::CachedTokens => "litellm_cached_tokens_metric",
            Self::ProviderCacheReadTokens => "litellm_provider_cache_read_input_tokens_metric",
            Self::ProviderCacheCreationTokens => {
                "litellm_provider_cache_creation_input_tokens_metric"
            }
            Self::McpToolCalls => "litellm_mcp_tool_calls_total",
            Self::McpToolCallSpend => "litellm_mcp_tool_call_spend_metric",
            Self::SpendCaptureRate => "litellm_spend_capture_rate",
        }
    }
}
