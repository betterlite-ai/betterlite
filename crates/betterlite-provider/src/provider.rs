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

// each stage matches the variants named once below, so a variant left out of that list fails to compile
macro_rules! dispatch_to_adapters {
    ($($variant:ident),+ $(,)?) => {
        impl Provider {
            #[must_use]
            pub const fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => <$variant as Adapter>::NAME,)+
                }
            }

            pub fn prepare(
                &self,
                request: &OperationRequest,
                deployment: &Deployment,
                catalog: &CatalogCapabilities,
            ) -> Result<PreparedRequest, OperationError> {
                match self {
                    $(Self::$variant(adapter) => adapter.prepare(request, deployment, catalog),)+
                }
            }

            #[must_use]
            pub fn execute<'call>(
                &'call self,
                client: &'call ProxiedClient,
                prepared: &'call PreparedRequest,
                deployment: &'call Deployment,
            ) -> Pin<Box<dyn Future<Output = Result<Upstream, OperationError>> + Send + 'call>> {
                match self {
                    $(Self::$variant(adapter) => Box::pin(adapter.execute(client, prepared, deployment)),)+
                }
            }

            pub fn decode(
                &self,
                prepared: &PreparedRequest,
                upstream: Upstream,
            ) -> Result<OperationResult, OperationError> {
                match self {
                    $(Self::$variant(adapter) => adapter.decode(prepared, upstream),)+
                }
            }
        }
    };
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
}

dispatch_to_adapters!(
    OpenAi,
    AiohttpOpenAi,
    Azure,
    AzureAi,
    Anthropic,
    Bedrock,
    VertexAi,
    Gemini,
    XaiMistral,
    EmbeddingRerank,
    Compatible,
    Legacy,
    SelfHosted,
    Subscription,
    MediaProviders,
    SpeechProviders,
);

mod tests;
