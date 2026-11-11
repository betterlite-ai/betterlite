use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider as DeploymentProvider};
use betterlite_openai::openai_base_config::CatalogCapabilities;

use crate::aiohttp_openai_dispatch::AiohttpOpenAi;
use crate::anthropic_dispatch::Anthropic;
use crate::azure_dispatch::Azure;
use crate::bedrock_dispatch_contract::Bedrock;
use crate::compatible_providers::Compatible;
use crate::embedding_rerank_providers::EmbeddingRerank;
use crate::foundry_dispatch::AzureAi;
use crate::legacy_dispatch::Legacy;
use crate::openai_dispatch::OpenAi;
use crate::operation::{OperationError, OperationRequest, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::self_hosted_dispatch::SelfHosted;
use crate::studio_dispatch::Gemini;
use crate::transform_retry::send_with_retry;
use crate::vertex_dispatch::VertexAi;
use crate::xai_mistral::XaiMistral;

pub mod media_providers;
pub mod speech_providers;
pub mod subscription;

use self::media_providers::MediaProviders;
use self::speech_providers::SpeechProviders;
use self::subscription::Subscription;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi(OpenAi),
    AiohttpOpenAi(AiohttpOpenAi),
    Azure(Azure),
    AzureAi(AzureAi),
    Anthropic(Anthropic),
    Bedrock(Bedrock),
    VertexAi(VertexAi),
    Gemini(Gemini),
    XaiMistral(XaiMistral),
    EmbeddingRerank(EmbeddingRerank),
    Compatible(Compatible),
    Legacy(Legacy),
    SelfHosted(SelfHosted),
    Subscription(Subscription),
    MediaProviders(MediaProviders),
    SpeechProviders(SpeechProviders),
}

impl Provider {
    // The deployment resolver names the identity; only the OpenAI identity has an executable adapter here besides aiohttp
    pub fn from_deployment(deployment: &ChatDeployment) -> Result<Self, OperationError> {
        match &deployment.provider {
            DeploymentProvider::OpenAi { .. } => Ok(Self::OpenAi(OpenAi)),
            DeploymentProvider::Unsupported { .. } => Self::from_slug(&deployment.litellm_model),
        }
    }

    fn from_slug(litellm_model: &str) -> Result<Self, OperationError> {
        let slug = litellm_model
            .split_once('/')
            .map_or(litellm_model, |(slug, _model)| slug);
        match slug {
            "openai" => Ok(Self::OpenAi(OpenAi)),
            "aiohttp_openai" => Ok(Self::AiohttpOpenAi(AiohttpOpenAi)),
            "azure" | "azure_text" => Ok(Self::Azure(Azure)),
            "azure_ai" => Ok(Self::AzureAi(AzureAi)),
            "anthropic" | "anthropic_text" => Ok(Self::Anthropic(Anthropic)),
            "bedrock" => Ok(Self::Bedrock(Bedrock)),
            "vertex_ai" | "vertex_ai_beta" => Ok(Self::VertexAi(VertexAi)),
            "gemini" => Ok(Self::Gemini(Gemini)),
            "xai" | "mistral" | "codestral" => Ok(Self::XaiMistral(XaiMistral)),
            other => Err(OperationError::UnavailableAdapter {
                provider: other.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::OpenAi(_) => "openai",
            Self::AiohttpOpenAi(_) => "aiohttp_openai",
            Self::Azure(_) => "azure",
            Self::AzureAi(_) => "azure_ai",
            Self::Anthropic(_) => "anthropic",
            Self::Bedrock(_) => "bedrock",
            Self::VertexAi(_) => "vertex_ai",
            Self::Gemini(_) => "gemini",
            Self::XaiMistral(_) => "xai_mistral",
            Self::EmbeddingRerank(_) => "embedding_rerank",
            Self::Compatible(_) => "compatible",
            Self::Legacy(_) => "legacy",
            Self::SelfHosted(_) => "self_hosted",
            Self::Subscription(_) => "subscription",
            Self::MediaProviders(_) => "media_providers",
            Self::SpeechProviders(_) => "speech_providers",
        }
    }

    pub fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        if let Self::OpenAi(provider) = self {
            return provider.prepare(request, deployment, catalog);
        }
        if let Self::AiohttpOpenAi(provider) = self {
            return provider.prepare(request, deployment, catalog);
        }
        Err(self.unavailable())
    }

    pub async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        if let Self::OpenAi(provider) = self {
            return send_with_retry(client, prepared, deployment, provider).await;
        }
        if let Self::AiohttpOpenAi(provider) = self {
            return send_with_retry(client, prepared, deployment, provider).await;
        }
        Err(self.unavailable())
    }

    pub fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        if let Self::OpenAi(provider) = self {
            return provider.decode(prepared, upstream);
        }
        if let Self::AiohttpOpenAi(provider) = self {
            return provider.decode(prepared, upstream);
        }
        Err(self.unavailable())
    }

    // A registered stub never runs a call; the typed answer names it and is returned before any HTTP work
    fn unavailable(self) -> OperationError {
        OperationError::UnavailableAdapter {
            provider: self.name().to_owned(),
        }
    }
}

mod tests;
