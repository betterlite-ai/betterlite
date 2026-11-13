use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};

pub const RESPONSES_ENDPOINT: &str = "/v1/responses";
pub const MESSAGES_ENDPOINT: &str = "/v1/messages";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderIdentity<'source> {
    Enum(ProviderId),
    Name(&'source str),
}

impl ProviderIdentity<'_> {
    fn normalize(self) -> (Option<ProviderId>, String) {
        match self {
            Self::Enum(provider) => (Some(provider), provider.to_string()),
            Self::Name(name) => (name.parse().ok(), name.to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponsesApiPolicy {
    Native,
    Json(ProviderId),
}

// get_provider_responses_api_config: the native switch runs first and a JSON descriptor only decides when it found nothing
pub fn responses_api_policy<Selector>(
    provider: ProviderIdentity<'_>,
    native: Selector,
    registry: &JSONProviderRegistry,
) -> Option<ResponsesApiPolicy>
where
    Selector: FnOnce(Option<ProviderId>) -> Option<ResponsesApiPolicy>,
{
    let (known, slug) = provider.normalize();
    if let Some(policy) = native(known) {
        return Some(policy);
    }
    registry
        .get(&slug)
        .filter(|descriptor| lists(descriptor, RESPONSES_ENDPOINT))
        .map(|descriptor| ResponsesApiPolicy::Json(descriptor.provider))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessagesApiPolicy {
    Deepseek,
    Json(ProviderId),
}

impl MessagesApiPolicy {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Deepseek => "DeepSeekAnthropicMessagesConfig",
            Self::Json(_) => "JSONProviderAnthropicMessagesConfig",
        }
    }
}

pub fn messages_api_policy(
    provider: ProviderId,
    registry: &JSONProviderRegistry,
) -> Option<MessagesApiPolicy> {
    if provider == ProviderId::Deepseek {
        return Some(MessagesApiPolicy::Deepseek);
    }
    registry
        .get(&provider.to_string())
        .filter(|descriptor| lists(descriptor, MESSAGES_ENDPOINT))
        .map(|descriptor| MessagesApiPolicy::Json(descriptor.provider))
}

// the embedding, rerank and image-generation arms each name the one config class they return
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatibleApiPolicy {
    CometapiEmbedding,
    DeepinfraRerank,
    AimlImageGeneration,
    CometapiImageGeneration,
}

impl CompatibleApiPolicy {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::CometapiEmbedding => "CometAPIEmbeddingConfig",
            Self::DeepinfraRerank => "DeepinfraRerankConfig",
            Self::AimlImageGeneration => "AimlImageGenerationConfig",
            Self::CometapiImageGeneration => "CometAPIImageGenerationConfig",
        }
    }
}

pub fn embedding_api_policy(provider: ProviderId) -> Option<CompatibleApiPolicy> {
    if provider == ProviderId::Cometapi {
        return Some(CompatibleApiPolicy::CometapiEmbedding);
    }
    None
}

pub fn rerank_api_policy(provider: ProviderId) -> Option<CompatibleApiPolicy> {
    if provider == ProviderId::Deepinfra {
        return Some(CompatibleApiPolicy::DeepinfraRerank);
    }
    None
}

pub fn image_generation_api_policy(provider: ProviderId) -> Option<CompatibleApiPolicy> {
    if provider == ProviderId::Aiml {
        return Some(CompatibleApiPolicy::AimlImageGeneration);
    }
    if provider == ProviderId::Cometapi {
        return Some(CompatibleApiPolicy::CometapiImageGeneration);
    }
    None
}

fn lists(descriptor: &ProviderDescriptor, endpoint: &str) -> bool {
    descriptor
        .config
        .supported_endpoints
        .iter()
        .any(|declared| declared == endpoint)
}

mod tests;
