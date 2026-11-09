use betterlite_models::catalog::ModelCatalog;

use super::base_url::validate_vertex_location;
use super::embedding::{EmbeddingUrlInput, get_embedding_url};
use super::error::VertexUrlError;
use super::model_name::get_model_for_vertex_ai_url;
use super::path::{VertexCollection, VertexPath, model_path, resource_path};
use super::{ApiVersion, Endpoint, UrlMode, VertexUrl};

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
        UrlMode::ImageGeneration => {
            let path = path_of(model, input)?;
            Ok(VertexUrl {
                url: model_path(&path, input.api_version, ApiVersion::V1, Endpoint::Predict),
                endpoint: Endpoint::Predict,
            })
        }
        UrlMode::CountTokens => count_tokens_url(model, input),
        UrlMode::BatchEmbedding => Err(VertexUrlError::UnsupportedMode {
            mode: UrlMode::BatchEmbedding,
        }),
    }
}

fn path_of<'model>(
    model: &'model str,
    input: &VertexUrlInput<'model>,
) -> Result<VertexPath<'model>, VertexUrlError> {
    let location = validate_vertex_location(input.location)?;
    let project = input.project.ok_or(VertexUrlError::ProjectRequired)?;
    Ok(VertexPath {
        model,
        project,
        location,
    })
}

fn chat_url(model: &str, input: &VertexUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let path = path_of(model, input)?;
    let (endpoint, suffix) = match input.stream {
        Some(true) => (Endpoint::StreamGenerateContent, "?alt=sse"),
        Some(false) | None => (Endpoint::GenerateContent, ""),
    };
    let mut url = model_path(&path, input.api_version, input.api_version, endpoint);
    url.push_str(suffix);
    Ok(VertexUrl { url, endpoint })
}

fn count_tokens_url(model: &str, input: &VertexUrlInput<'_>) -> Result<VertexUrl, VertexUrlError> {
    let path = path_of(model, input)?;
    Ok(VertexUrl {
        url: resource_path(
            &path,
            input.api_version,
            Endpoint::CountTokens,
            VertexCollection::Publishers,
        ),
        endpoint: Endpoint::CountTokens,
    })
}
