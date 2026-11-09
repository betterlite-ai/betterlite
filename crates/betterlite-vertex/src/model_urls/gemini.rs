use super::error::VertexUrlError;
use super::model_name::is_gemini_3_or_newer;
use super::{Endpoint, GeminiApiVersion, UrlMode, VertexUrl};

pub struct GeminiUrlInput<'model> {
    pub mode: UrlMode,
    pub model: &'model str,
    pub stream: Option<bool>,
}

pub fn get_gemini_url(input: &GeminiUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let api_version = if is_gemini_3_or_newer(input.model) {
        GeminiApiVersion::V1Alpha
    } else {
        GeminiApiVersion::V1Beta
    };
    let model_name = format!("models/{}", input.model);
    match input.mode {
        UrlMode::Chat => Ok(gemini_chat_url(api_version, &model_name, input.stream)),
        UrlMode::Embedding => Ok(gemini_v1beta_url(&model_name, Endpoint::EmbedContent)),
        UrlMode::BatchEmbedding => Ok(gemini_v1beta_url(&model_name, Endpoint::BatchEmbedContents)),
        UrlMode::CountTokens => Ok(gemini_v1beta_url(&model_name, Endpoint::CountTokens)),
        UrlMode::ImageGeneration => Err(VertexUrlError::GeminiImageGeneration),
    }
}

fn gemini_chat_url(
    api_version: GeminiApiVersion,
    model_name: &str,
    stream: Option<bool>,
) -> VertexUrl {
    let (endpoint, suffix) = match stream {
        Some(true) => (Endpoint::StreamGenerateContent, "?alt=sse"),
        Some(false) | None => (Endpoint::GenerateContent, ""),
    };
    let version = api_version.as_str();
    let endpoint_text = endpoint.as_str();
    VertexUrl {
        url: format!(
            "https://generativelanguage.googleapis.com/{version}/{model_name}:{endpoint_text}{suffix}"
        ),
        endpoint,
    }
}

fn gemini_v1beta_url(model_name: &str, endpoint: Endpoint) -> VertexUrl {
    let version = GeminiApiVersion::V1Beta.as_str();
    let endpoint_text = endpoint.as_str();
    VertexUrl {
        url: format!(
            "https://generativelanguage.googleapis.com/{version}/{model_name}:{endpoint_text}"
        ),
        endpoint,
    }
}
