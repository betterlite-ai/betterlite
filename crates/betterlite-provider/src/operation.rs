use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::{EmbeddingRequest, EmbeddingResponse};
use http::header::InvalidHeaderValue;
use serde::Serialize;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::aiohttp_openai_dispatch::{
    AiohttpChatResponse, AiohttpOpenAiRequest, AiohttpOpenAiResult,
};
use crate::anthropic_dispatch::{AnthropicRequest, AnthropicResult};
use crate::api::batch_records::{BatchRequest, BatchResult};
use crate::api::file_contracts::{FileRequest, FileResult};
use crate::api::image_contracts::{ImageRequest, ImageResult};
use crate::api::messages_native_prepare::{MessagesRequest, MessagesResult};
use crate::api::rerank_contracts::{RerankRequest, RerankResult};
use crate::api::responses_provider_contract::{ResponsesRequest, ResponsesResult};
use crate::api::speech_provider_contract::{SpeechRequest, SpeechResult};
use crate::api::transcription_contract::{TranscriptionRequest, TranscriptionResult};
use crate::azure_dispatch::{AzureRequest, AzureResult};
use crate::azure_media_dispatch::{AzureMediaRequest, AzureMediaResult};
use crate::azure_resource_dispatch::{AzureResourceRequest, AzureResourceResult};
use crate::bedrock_agent_dispatch::{BedrockAgentRequest, BedrockAgentResult};
use crate::bedrock_dispatch::{BedrockChatRequest, BedrockChatResult};
use crate::bedrock_dispatch_contract::{BedrockRequest, BedrockResult};
use crate::bedrock_image_dispatch::{BedrockImageRequest, BedrockImageResult};
use crate::bedrock_interactive_dispatch::{BedrockInteractiveRequest, BedrockInteractiveResult};
use crate::bedrock_other_dispatch::{BedrockOtherRequest, BedrockOtherResult};
use crate::bedrock_services_dispatch::{BedrockServicesRequest, BedrockServicesResult};
use crate::compatible_providers::{CompatibleRequest, CompatibleResult};
use crate::embedding_rerank_providers::{EmbeddingRerankRequest, EmbeddingRerankResult};
use crate::foundry_dispatch::{AzureAiRequest, AzureAiResult};
use crate::foundry_resource_dispatch::{FoundryResourceRequest, FoundryResourceResult};
use crate::legacy_dispatch::{LegacyRequest, LegacyResult};
use crate::openai_assistant_dispatch::{OpenAiAssistantRequest, OpenAiAssistantResult};
use crate::openai_dispatch::{OpenAiRequest, OpenAiResult};
use crate::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use crate::openai_resource_dispatch::{OpenAiResourceRequest, OpenAiResourceResult};
use crate::self_hosted_dispatch::{SelfHostedRequest, SelfHostedResult};
use crate::studio_dispatch::{GeminiRequest, GeminiResult};
use crate::transform_retry::TransformRetryError;
use crate::vertex_dispatch::{VertexAiRequest, VertexAiResult};
use crate::vertex_embedding_binding::{VertexEmbeddingRequest, VertexEmbeddingResult};
use crate::vertex_media_dispatch::{VertexMediaRequest, VertexMediaResult};
use crate::vertex_partner_dispatch::{VertexPartnerRequest, VertexPartnerResult};
use crate::vertex_service_dispatch::{VertexServiceRequest, VertexServiceResult};
use crate::xai_mistral::{XaiMistralRequest, XaiMistralResult};

#[derive(Debug, Clone)]
pub struct ChatOperation {
    pub model: String,
    pub request: ChatRequest,
    pub control: ControlOptions,
}

#[derive(Debug, Clone)]
pub struct EmbeddingOperation {
    pub model: String,
    pub request: EmbeddingRequest,
}

#[derive(Debug, Clone)]
pub enum OperationRequest {
    Chat(Box<ChatOperation>),
    Stream(Box<ChatOperation>),
    Embedding(Box<EmbeddingOperation>),
    AiohttpOpenAi(AiohttpOpenAiRequest),
    OpenAi(OpenAiRequest),
    Azure(AzureRequest),
    AzureAi(AzureAiRequest),
    Anthropic(AnthropicRequest),
    Bedrock(BedrockRequest),
    VertexAi(VertexAiRequest),
    Gemini(GeminiRequest),
    XaiMistral(XaiMistralRequest),
    EmbeddingRerank(EmbeddingRerankRequest),
    Compatible(CompatibleRequest),
    Legacy(LegacyRequest),
    SelfHosted(SelfHostedRequest),
    Image(ImageRequest),
    Rerank(RerankRequest),
    File(FileRequest),
    Batch(BatchRequest),
    Responses(ResponsesRequest),
    Transcription(TranscriptionRequest),
    Speech(SpeechRequest),
    Messages(MessagesRequest),
    OpenAiMedia(OpenAiMediaRequest),
    OpenAiResource(OpenAiResourceRequest),
    OpenAiAssistant(OpenAiAssistantRequest),
    AzureMedia(AzureMediaRequest),
    AzureResource(AzureResourceRequest),
    FoundryResource(FoundryResourceRequest),
    BedrockChat(BedrockChatRequest),
    BedrockOther(BedrockOtherRequest),
    BedrockServices(BedrockServicesRequest),
    BedrockImage(BedrockImageRequest),
    BedrockAgent(BedrockAgentRequest),
    BedrockInteractive(BedrockInteractiveRequest),
    VertexPartner(VertexPartnerRequest),
    VertexMedia(VertexMediaRequest),
    VertexService(VertexServiceRequest),
    VertexEmbedding(VertexEmbeddingRequest),
}

// The three operations the OpenAI-shaped adapters run; this one match is where a new request variant has to be placed
pub(crate) enum RequestKind<'request> {
    Chat(&'request ChatOperation),
    Stream(&'request ChatOperation),
    Embedding(&'request EmbeddingOperation),
    Other,
}

impl OperationRequest {
    #[rustfmt::skip]
    pub(crate) fn kind(&self) -> RequestKind<'_> {
        match self {
            Self::Chat(chat) => RequestKind::Chat(chat),
            Self::Stream(chat) => RequestKind::Stream(chat),
            Self::Embedding(embedding) => RequestKind::Embedding(embedding),
            Self::AiohttpOpenAi(_) | Self::OpenAi(_) | Self::Azure(_) | Self::AzureAi(_)
            | Self::Anthropic(_) | Self::Bedrock(_) | Self::VertexAi(_) | Self::Gemini(_)
            | Self::XaiMistral(_) | Self::EmbeddingRerank(_) | Self::Compatible(_) | Self::Legacy(_)
            | Self::SelfHosted(_) | Self::Image(_) | Self::Rerank(_) | Self::File(_)
            | Self::Batch(_) | Self::Responses(_) | Self::Transcription(_) | Self::Speech(_)
            | Self::Messages(_) | Self::OpenAiMedia(_) | Self::OpenAiResource(_) | Self::OpenAiAssistant(_)
            | Self::AzureMedia(_) | Self::AzureResource(_) | Self::FoundryResource(_) | Self::BedrockChat(_)
            | Self::BedrockOther(_) | Self::BedrockServices(_) | Self::BedrockImage(_) | Self::BedrockAgent(_)
            | Self::BedrockInteractive(_) | Self::VertexPartner(_) | Self::VertexMedia(_) | Self::VertexService(_)
            | Self::VertexEmbedding(_) => RequestKind::Other,
        }
    }
}

// The two chat projections keep their own shape: the aiohttp adapter copies only the fields LiteLLM copies
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChatCompletion {
    OpenAi(Box<ChatResponse>),
    AiohttpOpenAi(Box<AiohttpChatResponse>),
}

pub enum OperationResult {
    Chat(ChatCompletion),
    Embedding(Box<EmbeddingResponse>),
    Stream(Box<StreamResponse>),
    AiohttpOpenAi(AiohttpOpenAiResult),
    OpenAi(OpenAiResult),
    Azure(AzureResult),
    AzureAi(AzureAiResult),
    Anthropic(AnthropicResult),
    Bedrock(BedrockResult),
    VertexAi(VertexAiResult),
    Gemini(GeminiResult),
    XaiMistral(XaiMistralResult),
    EmbeddingRerank(EmbeddingRerankResult),
    Compatible(CompatibleResult),
    Legacy(LegacyResult),
    SelfHosted(SelfHostedResult),
    Image(ImageResult),
    Rerank(RerankResult),
    File(FileResult),
    Batch(BatchResult),
    Responses(ResponsesResult),
    Transcription(TranscriptionResult),
    Speech(SpeechResult),
    Messages(MessagesResult),
    OpenAiMedia(OpenAiMediaResult),
    OpenAiResource(OpenAiResourceResult),
    OpenAiAssistant(OpenAiAssistantResult),
    AzureMedia(AzureMediaResult),
    AzureResource(AzureResourceResult),
    FoundryResource(FoundryResourceResult),
    BedrockChat(BedrockChatResult),
    BedrockOther(BedrockOtherResult),
    BedrockServices(BedrockServicesResult),
    BedrockImage(BedrockImageResult),
    BedrockAgent(BedrockAgentResult),
    BedrockInteractive(BedrockInteractiveResult),
    VertexPartner(VertexPartnerResult),
    VertexMedia(VertexMediaResult),
    VertexService(VertexServiceResult),
    VertexEmbedding(VertexEmbeddingResult),
}

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("betterlite has no executable adapter for the {provider} provider")]
    UnavailableAdapter { provider: String },
    #[error("betterlite does not run the {operation} operation through the {provider} provider")]
    UnsupportedOperation { provider: String, operation: String },
    #[error("the provider answered with status {}", .response.status)]
    Provider { response: Box<UpstreamResponse> },
    #[error("the provider could not be reached: {source}")]
    Http {
        #[source]
        source: TransportError,
    },
    #[error("the provider did not answer within the deployment timeout")]
    Timeout,
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: JsonError,
    },
    #[error("the provider answer is not the expected {operation} response: {source}")]
    Decode {
        operation: &'static str,
        #[source]
        source: JsonError,
    },
    #[error("the provider answer is larger than the {limit} bytes betterlite accepts")]
    TooLarge { limit: usize },
    #[error("no API key is set for this deployment")]
    MissingApiKey,
    #[error("the {name} header holds a value HTTP cannot carry: {source}")]
    InvalidHeader {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    TransformRetry(#[from] TransformRetryError),
}

// LiteLLM routes a status of 400 and up back as the provider's own error body, so the caller can render it
pub(crate) fn success(response: UpstreamResponse) -> Result<UpstreamResponse, OperationError> {
    if response.status >= 400 {
        return Err(OperationError::Provider {
            response: Box::new(response),
        });
    }
    Ok(response)
}

mod tests;
