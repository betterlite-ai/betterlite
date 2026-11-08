use super::error::VertexUrlError;
use super::model_name::is_gemini_3_or_newer;
use super::{UrlMode, VertexUrl};

pub struct GeminiUrlInput<'model> {
    pub mode: UrlMode,
    pub model: &'model str,
    pub stream: Option<bool>,
}

pub fn get_gemini_url(input: &GeminiUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let api_version = if is_gemini_3_or_newer(input.model) {
        "v1alpha"
    } else {
        "v1beta"
    };
    let model_name = format!("models/{}", input.model);
    match input.mode {
        UrlMode::Chat => Ok(gemini_chat_url(api_version, &model_name, input.stream)),
        UrlMode::Embedding => Ok(gemini_v1beta_url(&model_name, "embedContent")),
        UrlMode::BatchEmbedding => Ok(gemini_v1beta_url(&model_name, "batchEmbedContents")),
        UrlMode::CountTokens => Ok(gemini_v1beta_url(&model_name, "countTokens")),
        UrlMode::ImageGeneration => Err(VertexUrlError::GeminiImageGeneration),
    }
}

fn gemini_chat_url(api_version: &str, model_name: &str, stream: Option<bool>) -> VertexUrl {
    let (endpoint, suffix) = match stream {
        Some(true) => ("streamGenerateContent", "?alt=sse"),
        Some(false) | None => ("generateContent", ""),
    };
    VertexUrl {
        url: format!(
            "https://generativelanguage.googleapis.com/{api_version}/{model_name}:{endpoint}{suffix}"
        ),
        endpoint,
    }
}

fn gemini_v1beta_url(model_name: &str, endpoint: &'static str) -> VertexUrl {
    VertexUrl {
        url: format!("https://generativelanguage.googleapis.com/v1beta/{model_name}:{endpoint}"),
        endpoint,
    }
}
