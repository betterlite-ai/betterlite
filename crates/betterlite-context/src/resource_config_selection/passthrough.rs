use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughConfigTag {
    Bedrock,
    BedrockMantle,
    VLLM,
    Azure,
    AzureAI,
    GigaChat,
    Watsonx,
    NvidiaNim,
}

const DIRECT_TAGS: &[(ProviderId, PassthroughConfigTag)] = &[
    (ProviderId::Bedrock, PassthroughConfigTag::Bedrock),
    (
        ProviderId::BedrockMantle,
        PassthroughConfigTag::BedrockMantle,
    ),
    (ProviderId::Vllm, PassthroughConfigTag::VLLM),
    (ProviderId::HostedVllm, PassthroughConfigTag::VLLM),
    (ProviderId::Azure, PassthroughConfigTag::Azure),
    (ProviderId::AzureAi, PassthroughConfigTag::AzureAI),
    (ProviderId::Gigachat, PassthroughConfigTag::GigaChat),
    (ProviderId::Watsonx, PassthroughConfigTag::Watsonx),
    (ProviderId::NvidiaNim, PassthroughConfigTag::NvidiaNim),
];

#[must_use]
pub fn select_passthrough_config(
    _model: &str,
    provider: ProviderId,
) -> Option<PassthroughConfigTag> {
    DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchesConfigTag {
    Bedrock,
    Mistral,
}

const BATCH_DIRECT_TAGS: &[(ProviderId, BatchesConfigTag)] = &[
    (ProviderId::Bedrock, BatchesConfigTag::Bedrock),
    (ProviderId::Mistral, BatchesConfigTag::Mistral),
];

#[must_use]
pub fn select_batches_config(_model: &str, provider: ProviderId) -> Option<BatchesConfigTag> {
    BATCH_DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
}
