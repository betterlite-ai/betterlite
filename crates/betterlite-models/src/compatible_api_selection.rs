use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::JSONProviderRegistry;

pub const RESPONSES_ENDPOINT: &str = "/v1/responses";
pub const MESSAGES_ENDPOINT: &str = "/v1/messages";

// LiteLLM accepts the enum or its wire name; a name that parses to no member still reaches the registry lookup
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

// DeepSeek answers Messages natively whatever the registry says; the JSON tail needs the exact endpoint, after that arm
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddingApiPolicy {
    Cometapi,
}

impl EmbeddingApiPolicy {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Cometapi => "CometAPIEmbeddingConfig",
        }
    }
}

pub fn embedding_api_policy(provider: ProviderId) -> Option<EmbeddingApiPolicy> {
    if provider == ProviderId::Cometapi {
        return Some(EmbeddingApiPolicy::Cometapi);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerankApiPolicy {
    Deepinfra,
}

impl RerankApiPolicy {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Deepinfra => "DeepinfraRerankConfig",
        }
    }
}

pub fn rerank_api_policy(provider: ProviderId) -> Option<RerankApiPolicy> {
    if provider == ProviderId::Deepinfra {
        return Some(RerankApiPolicy::Deepinfra);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageGenerationApiPolicy {
    Aiml,
    Cometapi,
}

impl ImageGenerationApiPolicy {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Aiml => "AimlImageGenerationConfig",
            Self::Cometapi => "CometAPIImageGenerationConfig",
        }
    }
}

// both image factories ignore the model and return their one config class
pub fn image_generation_api_policy(provider: ProviderId) -> Option<ImageGenerationApiPolicy> {
    if provider == ProviderId::Aiml {
        return Some(ImageGenerationApiPolicy::Aiml);
    }
    if provider == ProviderId::Cometapi {
        return Some(ImageGenerationApiPolicy::Cometapi);
    }
    None
}

fn lists(descriptor: &crate::json_provider_registry::ProviderDescriptor, endpoint: &str) -> bool {
    descriptor
        .config
        .supported_endpoints
        .iter()
        .any(|declared| declared == endpoint)
}

mod tests;
