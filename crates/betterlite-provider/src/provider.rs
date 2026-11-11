use std::future::{Future, ready};
use std::pin::Pin;

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

// The three stages every provider answers; a stub keeps these defaults until its owner overrides them in its own module
pub trait Adapter {
    const NAME: &'static str;

    fn prepare(
        &self,
        _request: &OperationRequest,
        _deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        Err(OperationError::UnavailableAdapter {
            provider: Self::NAME.to_owned(),
        })
    }

    fn execute(
        &self,
        _client: &ProxiedClient,
        _prepared: &PreparedRequest,
        _deployment: &Deployment,
    ) -> impl Future<Output = Result<Upstream, OperationError>> + Send {
        ready(Err(OperationError::UnavailableAdapter {
            provider: Self::NAME.to_owned(),
        }))
    }

    fn decode(
        &self,
        _prepared: &PreparedRequest,
        _upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        Err(OperationError::UnavailableAdapter {
            provider: Self::NAME.to_owned(),
        })
    }
}

impl Provider {
    // The model the resolver keeps carries the slug that picks aiohttp or a stub; OpenAI is named by the resolver itself
    pub fn from_deployment(deployment: &ChatDeployment) -> Result<Self, OperationError> {
        match &deployment.provider {
            DeploymentProvider::OpenAi { .. } => Ok(Self::OpenAi(OpenAi)),
            DeploymentProvider::Unsupported { model } => Self::from_slug(
                model
                    .split_once('/')
                    .map_or(model.as_str(), |(slug, _model)| slug),
            ),
        }
    }

    pub(crate) fn from_slug(slug: &str) -> Result<Self, OperationError> {
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
            Self::OpenAi(_) => OpenAi::NAME,
            Self::AiohttpOpenAi(_) => AiohttpOpenAi::NAME,
            Self::Azure(_) => Azure::NAME,
            Self::AzureAi(_) => AzureAi::NAME,
            Self::Anthropic(_) => Anthropic::NAME,
            Self::Bedrock(_) => Bedrock::NAME,
            Self::VertexAi(_) => VertexAi::NAME,
            Self::Gemini(_) => Gemini::NAME,
            Self::XaiMistral(_) => XaiMistral::NAME,
            Self::EmbeddingRerank(_) => EmbeddingRerank::NAME,
            Self::Compatible(_) => Compatible::NAME,
            Self::Legacy(_) => Legacy::NAME,
            Self::SelfHosted(_) => SelfHosted::NAME,
            Self::Subscription(_) => Subscription::NAME,
            Self::MediaProviders(_) => MediaProviders::NAME,
            Self::SpeechProviders(_) => SpeechProviders::NAME,
        }
    }

    pub fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        match self {
            Self::OpenAi(adapter) => adapter.prepare(request, deployment, catalog),
            Self::AiohttpOpenAi(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Azure(adapter) => adapter.prepare(request, deployment, catalog),
            Self::AzureAi(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Anthropic(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Bedrock(adapter) => adapter.prepare(request, deployment, catalog),
            Self::VertexAi(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Gemini(adapter) => adapter.prepare(request, deployment, catalog),
            Self::XaiMistral(adapter) => adapter.prepare(request, deployment, catalog),
            Self::EmbeddingRerank(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Compatible(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Legacy(adapter) => adapter.prepare(request, deployment, catalog),
            Self::SelfHosted(adapter) => adapter.prepare(request, deployment, catalog),
            Self::Subscription(adapter) => adapter.prepare(request, deployment, catalog),
            Self::MediaProviders(adapter) => adapter.prepare(request, deployment, catalog),
            Self::SpeechProviders(adapter) => adapter.prepare(request, deployment, catalog),
        }
    }

    // boxed so the sixteen arms stay a plain match; one request allocates one future
    #[must_use]
    pub fn execute<'call>(
        &'call self,
        client: &'call ProxiedClient,
        prepared: &'call PreparedRequest,
        deployment: &'call Deployment,
    ) -> Pin<Box<dyn Future<Output = Result<Upstream, OperationError>> + Send + 'call>> {
        match self {
            Self::OpenAi(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::AiohttpOpenAi(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Azure(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::AzureAi(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Anthropic(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Bedrock(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::VertexAi(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Gemini(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::XaiMistral(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::EmbeddingRerank(adapter) => {
                Box::pin(adapter.execute(client, prepared, deployment))
            }
            Self::Compatible(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Legacy(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::SelfHosted(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::Subscription(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),
            Self::MediaProviders(adapter) => {
                Box::pin(adapter.execute(client, prepared, deployment))
            }
            Self::SpeechProviders(adapter) => {
                Box::pin(adapter.execute(client, prepared, deployment))
            }
        }
    }

    pub fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        match self {
            Self::OpenAi(adapter) => adapter.decode(prepared, upstream),
            Self::AiohttpOpenAi(adapter) => adapter.decode(prepared, upstream),
            Self::Azure(adapter) => adapter.decode(prepared, upstream),
            Self::AzureAi(adapter) => adapter.decode(prepared, upstream),
            Self::Anthropic(adapter) => adapter.decode(prepared, upstream),
            Self::Bedrock(adapter) => adapter.decode(prepared, upstream),
            Self::VertexAi(adapter) => adapter.decode(prepared, upstream),
            Self::Gemini(adapter) => adapter.decode(prepared, upstream),
            Self::XaiMistral(adapter) => adapter.decode(prepared, upstream),
            Self::EmbeddingRerank(adapter) => adapter.decode(prepared, upstream),
            Self::Compatible(adapter) => adapter.decode(prepared, upstream),
            Self::Legacy(adapter) => adapter.decode(prepared, upstream),
            Self::SelfHosted(adapter) => adapter.decode(prepared, upstream),
            Self::Subscription(adapter) => adapter.decode(prepared, upstream),
            Self::MediaProviders(adapter) => adapter.decode(prepared, upstream),
            Self::SpeechProviders(adapter) => adapter.decode(prepared, upstream),
        }
    }
}

mod tests;
