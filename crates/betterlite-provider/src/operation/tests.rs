#![cfg(test)]

use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_openai::openai_base_config::CatalogCapabilities;
use rstest::rstest;
use url::Url;

use super::{
    AiohttpOpenAiRequest, AnthropicRequest, AzureAiRequest, AzureMediaRequest, AzureRequest,
    AzureResourceRequest, BatchRequest, BedrockAgentRequest, BedrockChatRequest,
    BedrockImageRequest, BedrockInteractiveRequest, BedrockOtherRequest, BedrockRequest,
    BedrockServicesRequest, CompatibleRequest, EmbeddingRerankRequest, FileRequest,
    FoundryResourceRequest, GeminiRequest, ImageRequest, LegacyRequest, MessagesRequest,
    OpenAiAssistantRequest, OpenAiMediaRequest, OpenAiRequest, OpenAiResourceRequest,
    OperationError, OperationRequest, RerankRequest, ResponsesRequest, SelfHostedRequest,
    SpeechRequest, TranscriptionRequest, VertexAiRequest, VertexEmbeddingRequest,
    VertexMediaRequest, VertexPartnerRequest, VertexServiceRequest, XaiMistralRequest,
};
use crate::aiohttp_openai_dispatch::AiohttpOpenAi;
use crate::openai_dispatch::OpenAi;
use crate::prepared::deployment::Deployment;

use crate::provider::Adapter as _;
mod status;

fn deployment() -> Deployment {
    Deployment {
        model: String::from("up"),
        api_base: String::from("https://host"),
        endpoint: Url::parse("https://host/v1/chat/completions").unwrap(),
        api_key: None,
        authorization: None,
        organization: None,
        timeout: Duration::from_secs(30),
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

#[rstest]
#[case::aiohttp_open_ai(OperationRequest::AiohttpOpenAi(AiohttpOpenAiRequest))]
#[case::open_ai(OperationRequest::OpenAi(OpenAiRequest))]
#[case::azure(OperationRequest::Azure(AzureRequest))]
#[case::azure_ai(OperationRequest::AzureAi(AzureAiRequest))]
#[case::anthropic(OperationRequest::Anthropic(AnthropicRequest))]
#[case::bedrock(OperationRequest::Bedrock(BedrockRequest))]
#[case::vertex_ai(OperationRequest::VertexAi(VertexAiRequest))]
#[case::gemini(OperationRequest::Gemini(GeminiRequest))]
#[case::xai_mistral(OperationRequest::XaiMistral(XaiMistralRequest))]
#[case::embedding_rerank(OperationRequest::EmbeddingRerank(EmbeddingRerankRequest))]
#[case::compatible(OperationRequest::Compatible(CompatibleRequest))]
#[case::legacy(OperationRequest::Legacy(LegacyRequest))]
#[case::self_hosted(OperationRequest::SelfHosted(SelfHostedRequest))]
#[case::image(OperationRequest::Image(ImageRequest))]
#[case::rerank(OperationRequest::Rerank(RerankRequest))]
#[case::file(OperationRequest::File(FileRequest))]
#[case::batch(OperationRequest::Batch(BatchRequest))]
#[case::responses(OperationRequest::Responses(ResponsesRequest))]
#[case::transcription(OperationRequest::Transcription(TranscriptionRequest))]
#[case::speech(OperationRequest::Speech(SpeechRequest))]
#[case::messages(OperationRequest::Messages(MessagesRequest))]
#[case::open_ai_media(OperationRequest::OpenAiMedia(OpenAiMediaRequest))]
#[case::open_ai_resource(OperationRequest::OpenAiResource(OpenAiResourceRequest))]
#[case::open_ai_assistant(OperationRequest::OpenAiAssistant(OpenAiAssistantRequest))]
#[case::azure_media(OperationRequest::AzureMedia(AzureMediaRequest))]
#[case::azure_resource(OperationRequest::AzureResource(AzureResourceRequest))]
#[case::foundry_resource(OperationRequest::FoundryResource(FoundryResourceRequest))]
#[case::bedrock_chat(OperationRequest::BedrockChat(BedrockChatRequest))]
#[case::bedrock_other(OperationRequest::BedrockOther(BedrockOtherRequest))]
#[case::bedrock_services(OperationRequest::BedrockServices(BedrockServicesRequest))]
#[case::bedrock_image(OperationRequest::BedrockImage(BedrockImageRequest))]
#[case::bedrock_agent(OperationRequest::BedrockAgent(BedrockAgentRequest))]
#[case::bedrock_interactive(OperationRequest::BedrockInteractive(BedrockInteractiveRequest))]
#[case::vertex_partner(OperationRequest::VertexPartner(VertexPartnerRequest))]
#[case::vertex_media(OperationRequest::VertexMedia(VertexMediaRequest))]
#[case::vertex_service(OperationRequest::VertexService(VertexServiceRequest))]
#[case::vertex_embedding(OperationRequest::VertexEmbedding(VertexEmbeddingRequest))]
fn provider_framework_tests_behavior(#[case] operation: OperationRequest) {
    let first = OpenAi
        .prepare(&operation, &deployment(), &catalog())
        .unwrap_err();
    let second = AiohttpOpenAi
        .prepare(&operation, &deployment(), &catalog())
        .unwrap_err();
    for refused in [first, second] {
        assert!(matches!(
            refused,
            OperationError::UnsupportedOperation { .. }
        ));
    }
}
