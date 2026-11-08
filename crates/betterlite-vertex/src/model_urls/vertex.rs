use betterlite_models::catalog::ModelCatalog;

use super::base_url::{host_for, validate_vertex_location};
use super::embedding::{EmbeddingUrlInput, get_embedding_url};
use super::error::VertexUrlError;
use super::model_name::{get_model_for_vertex_ai_url, is_decimal};
use super::{ApiVersion, UrlMode, VertexUrl};

pub struct VertexUrlInput<'model> {
    pub mode: UrlMode,
    pub model: &'model str,
    pub stream: Option<bool>,
    pub project: Option<&'model str>,
    pub location: Option<&'model str>,
    pub api_version: ApiVersion,
}

pub fn get_vertex_url(
    catalog: &ModelCatalog,
    input: &VertexUrlInput<'_>,
) -> Result<VertexUrl, VertexUrlError> {
    let model = get_model_for_vertex_ai_url(input.model);
    match input.mode {
        UrlMode::Chat => chat_url(model, input),
        UrlMode::Embedding => get_embedding_url(
            catalog,
            &EmbeddingUrlInput {
                model,
                project: input.project,
                location: input.location,
                api_version: input.api_version,
            },
        ),
        UrlMode::ImageGeneration => image_generation_url(model, input),
        UrlMode::CountTokens => count_tokens_url(model, input),
        UrlMode::BatchEmbedding => Err(VertexUrlError::UnsupportedMode {
            mode: "batch_embedding",
        }),
    }
}

fn chat_url(model: &str, input: &VertexUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let location = validate_vertex_location(input.location)?;
    let project = input.project.ok_or(VertexUrlError::ProjectRequired)?;
    let (endpoint, suffix) = match input.stream {
        Some(true) => ("streamGenerateContent", "?alt=sse"),
        Some(false) | None => ("generateContent", ""),
    };
    let base_url = host_for(location);
    let version = input.api_version.as_str();
    let mut url = if is_decimal(model) {
        format!(
            "{base_url}/{version}/projects/{project}/locations/{location}/endpoints/{model}:{endpoint}"
        )
    } else {
        format!(
            "{base_url}/{version}/projects/{project}/locations/{location}/publishers/google/models/{model}:{endpoint}"
        )
    };
    url.push_str(suffix);
    Ok(VertexUrl { url, endpoint })
}

fn image_generation_url(
    model: &str,
    input: &VertexUrlInput<'_>,
) -> Result<VertexUrl, VertexUrlError> {
    let location = validate_vertex_location(input.location)?;
    let project = input.project.ok_or(VertexUrlError::ProjectRequired)?;
    let base_url = host_for(location);
    let version = input.api_version.as_str();
    let url = if is_decimal(model) {
        format!(
            "{base_url}/{version}/projects/{project}/locations/{location}/endpoints/{model}:predict"
        )
    } else {
        format!(
            "{base_url}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:predict"
        )
    };
    Ok(VertexUrl {
        url,
        endpoint: "predict",
    })
}

fn count_tokens_url(model: &str, input: &VertexUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let location = validate_vertex_location(input.location)?;
    let project = input.project.ok_or(VertexUrlError::ProjectRequired)?;
    let base_url = host_for(location);
    let version = input.api_version.as_str();
    let url = format!(
        "{base_url}/{version}/projects/{project}/locations/{location}/publishers/google/models/{model}:countTokens"
    );
    Ok(VertexUrl {
        url,
        endpoint: "countTokens",
    })
}
