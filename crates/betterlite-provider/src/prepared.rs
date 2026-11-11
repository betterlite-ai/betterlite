use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use http::{HeaderMap, Method};
use url::Url;

use crate::aiohttp_openai_dispatch::AiohttpOpenAiPrepared;
use crate::anthropic_dispatch::AnthropicPrepared;
use crate::api::batch_records::BatchPrepared;
use crate::api::file_contracts::FilePrepared;
use crate::api::image_contracts::ImagePrepared;
use crate::api::messages_native_prepare::MessagesPrepared;
use crate::api::rerank_contracts::RerankPrepared;
use crate::api::responses_provider_contract::ResponsesPrepared;
use crate::api::speech_provider_contract::SpeechPrepared;
use crate::api::transcription_contract::TranscriptionPrepared;
use crate::azure_dispatch::AzurePrepared;
use crate::azure_media_dispatch::AzureMediaPrepared;
use crate::azure_resource_dispatch::AzureResourcePrepared;
use crate::bedrock_agent_dispatch::BedrockAgentPrepared;
use crate::bedrock_dispatch::BedrockChatPrepared;
use crate::bedrock_dispatch_contract::BedrockPrepared;
use crate::bedrock_image_dispatch::BedrockImagePrepared;
use crate::bedrock_interactive_dispatch::BedrockInteractivePrepared;
use crate::bedrock_other_dispatch::BedrockOtherPrepared;
use crate::bedrock_services_dispatch::BedrockServicesPrepared;
use crate::compatible_providers::CompatiblePrepared;
use crate::embedding_rerank_providers::EmbeddingRerankPrepared;
use crate::foundry_dispatch::AzureAiPrepared;
use crate::foundry_resource_dispatch::FoundryResourcePrepared;
use crate::legacy_dispatch::LegacyPrepared;
use crate::openai_assistant_dispatch::OpenAiAssistantPrepared;
use crate::openai_dispatch::OpenAiPrepared;
use crate::openai_media_dispatch::OpenAiMediaPrepared;
use crate::openai_resource_dispatch::OpenAiResourcePrepared;
use crate::operation::OperationError;
use crate::self_hosted_dispatch::SelfHostedPrepared;
use crate::studio_dispatch::GeminiPrepared;
use crate::transform_retry::TransformRetry;
use crate::vertex_dispatch::VertexAiPrepared;
use crate::vertex_embedding_binding::VertexEmbeddingPrepared;
use crate::vertex_media_dispatch::VertexMediaPrepared;
use crate::vertex_partner_dispatch::VertexPartnerPrepared;
use crate::vertex_service_dispatch::VertexServicePrepared;
use crate::xai_mistral::XaiMistralPrepared;

#[derive(Debug, Clone)]
pub struct PreparedHttp {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
    pub body_limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PreparedChat {
    pub http: PreparedHttp,
    pub retry: TransformRetry,
    pub client_model: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PreparedRequest {
    Chat(Box<PreparedChat>),
    Stream(Box<PreparedChat>),
    Embedding(Box<PreparedChat>),
    AiohttpOpenAi(AiohttpOpenAiPrepared),
    OpenAi(OpenAiPrepared),
    Azure(AzurePrepared),
    AzureAi(AzureAiPrepared),
    Anthropic(AnthropicPrepared),
    Bedrock(BedrockPrepared),
    VertexAi(VertexAiPrepared),
    Gemini(GeminiPrepared),
    XaiMistral(XaiMistralPrepared),
    EmbeddingRerank(EmbeddingRerankPrepared),
    Compatible(CompatiblePrepared),
    Legacy(LegacyPrepared),
    SelfHosted(SelfHostedPrepared),
    Image(ImagePrepared),
    Rerank(RerankPrepared),
    File(FilePrepared),
    Batch(BatchPrepared),
    Responses(ResponsesPrepared),
    Transcription(TranscriptionPrepared),
    Speech(SpeechPrepared),
    Messages(MessagesPrepared),
    OpenAiMedia(OpenAiMediaPrepared),
    OpenAiResource(OpenAiResourcePrepared),
    OpenAiAssistant(OpenAiAssistantPrepared),
    AzureMedia(AzureMediaPrepared),
    AzureResource(AzureResourcePrepared),
    FoundryResource(FoundryResourcePrepared),
    BedrockChat(BedrockChatPrepared),
    BedrockOther(BedrockOtherPrepared),
    BedrockServices(BedrockServicesPrepared),
    BedrockImage(BedrockImagePrepared),
    BedrockAgent(BedrockAgentPrepared),
    BedrockInteractive(BedrockInteractivePrepared),
    VertexPartner(VertexPartnerPrepared),
    VertexMedia(VertexMediaPrepared),
    VertexService(VertexServicePrepared),
    VertexEmbedding(VertexEmbeddingPrepared),
}

// The prepared calls that hold an HTTP request; this one match is where a new prepared variant has to be placed
pub(crate) enum PreparedKind<'prepared> {
    Chat(&'prepared PreparedChat),
    Stream(&'prepared PreparedChat),
    Embedding(&'prepared PreparedChat),
    Other,
}

impl PreparedRequest {
    #[rustfmt::skip]
    pub(crate) fn kind(&self) -> PreparedKind<'_> {
        match self {
            Self::Chat(prepared) => PreparedKind::Chat(prepared),
            Self::Stream(prepared) => PreparedKind::Stream(prepared),
            Self::Embedding(prepared) => PreparedKind::Embedding(prepared),
            Self::AiohttpOpenAi(_) | Self::OpenAi(_) | Self::Azure(_) | Self::AzureAi(_)
            | Self::Anthropic(_) | Self::Bedrock(_) | Self::VertexAi(_) | Self::Gemini(_)
            | Self::XaiMistral(_) | Self::EmbeddingRerank(_) | Self::Compatible(_) | Self::Legacy(_)
            | Self::SelfHosted(_) | Self::Image(_) | Self::Rerank(_) | Self::File(_)
            | Self::Batch(_) | Self::Responses(_) | Self::Transcription(_) | Self::Speech(_)
            | Self::Messages(_) | Self::OpenAiMedia(_) | Self::OpenAiResource(_) | Self::OpenAiAssistant(_)
            | Self::AzureMedia(_) | Self::AzureResource(_) | Self::FoundryResource(_) | Self::BedrockChat(_)
            | Self::BedrockOther(_) | Self::BedrockServices(_) | Self::BedrockImage(_) | Self::BedrockAgent(_)
            | Self::BedrockInteractive(_) | Self::VertexPartner(_) | Self::VertexMedia(_) | Self::VertexService(_)
            | Self::VertexEmbedding(_) => PreparedKind::Other,
        }
    }

    #[must_use]
    pub fn retry(&self) -> TransformRetry {
        match self.kind() {
            PreparedKind::Chat(prepared)
            | PreparedKind::Stream(prepared)
            | PreparedKind::Embedding(prepared) => prepared.retry,
            PreparedKind::Other => TransformRetry::DISABLED,
        }
    }

    fn http(&self) -> Option<&PreparedHttp> {
        match self.kind() {
            PreparedKind::Chat(prepared)
            | PreparedKind::Stream(prepared)
            | PreparedKind::Embedding(prepared) => Some(&prepared.http),
            PreparedKind::Other => None,
        }
    }

    pub(crate) async fn send(&self, client: &ProxiedClient) -> Result<Upstream, OperationError> {
        let Some(http) = self.http() else {
            return Err(OperationError::UnsupportedOperation {
                provider: String::new(),
                operation: String::new(),
            });
        };
        let request = OutboundRequest {
            method: http.method.clone(),
            url: http.url.clone(),
            query: Vec::new(),
            headers: http.headers.clone(),
            body: RequestBody::Json(http.body.clone()),
            deadline: http.deadline,
        };
        let response = client
            .send_following(request)
            .await
            .map_err(OperationError::from)?;
        let limit = http.body_limit.unwrap_or(client.limits.body_bytes);
        if matches!(self, Self::Stream(_)) && response.response.status().is_success() {
            return Ok(Upstream::Streaming(response));
        }
        let buffered = response
            .buffered(limit)
            .await
            .map_err(OperationError::from)?;
        Ok(Upstream::Buffered(buffered))
    }
}

pub enum Upstream {
    Buffered(UpstreamResponse),
    Streaming(StreamResponse),
}

impl Upstream {
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        match self {
            Self::Buffered(response) => &response.headers,
            Self::Streaming(response) => response.response.headers(),
        }
    }
}

pub(crate) mod body;
pub mod deployment;

mod tests;
