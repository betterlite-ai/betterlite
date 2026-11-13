use betterlite_models::identity::provider_id::ProviderId;

// LiteLLM's get_supported_anthropic_messages_params order; metadata and stream are deliberately absent
const NATIVE_FIELDS: [&str; 19] = [
    "messages",
    "model",
    "system",
    "max_tokens",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "context_management",
    "compaction",
    "output_format",
    "inference_geo",
    "speed",
    "output_config",
    "reasoning_effort",
    "safeguards",
];

const COMPACTION: &str = "compaction";

#[must_use]
pub fn native_supported_fields(provider: Option<ProviderId>) -> Vec<&'static str> {
    let keeps_compaction = matches!(provider, None | Some(ProviderId::Anthropic));
    NATIVE_FIELDS
        .iter()
        .copied()
        .filter(|field| keeps_compaction || *field != COMPACTION)
        .collect()
}

// A null provider resolves to anthropic, so only a named cross-provider target filters native beta headers
#[must_use]
pub const fn native_filters_betas(provider: Option<ProviderId>) -> bool {
    match provider {
        None | Some(ProviderId::Anthropic) => false,
        Some(_) => true,
    }
}
