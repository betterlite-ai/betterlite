use betterlite_models::catalog::ModelCatalog;

use super::base_url::{host_for, validate_vertex_location};
use super::error::VertexUrlError;
use super::model_name::{get_vertex_base_model_name, is_decimal};
use super::{ApiVersion, VertexUrl};

pub struct EmbeddingUrlInput<'model> {
    pub model: &'model str,
    pub project: Option<&'model str>,
    pub location: Option<&'model str>,
    pub api_version: ApiVersion,
}

pub fn get_embedding_url(
    catalog: &ModelCatalog,
    input: &EmbeddingUrlInput<'_>,
) -> Result<VertexUrl, VertexUrlError> {
    let location = validate_vertex_location(input.location)?;
    let project = input.project.ok_or(VertexUrlError::ProjectRequired)?;
    let model = get_vertex_base_model_name(input.model);
    let endpoint = if uses_embed_content(catalog, input.model) {
        "embedContent"
    } else {
        "predict"
    };
    let base_url = host_for(location);
    let version = input.api_version.as_str();
    let url = if is_decimal(model) {
        format!(
            "{base_url}/{version}/projects/{project}/locations/{location}/endpoints/{model}:{endpoint}"
        )
    } else {
        format!(
            "{base_url}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:{endpoint}"
        )
    };
    Ok(VertexUrl { url, endpoint })
}

// get_model_info with custom_llm_provider reads vertex_ai/<model> before <model>
fn uses_embed_content(catalog: &ModelCatalog, model: &str) -> bool {
    let prefixed = format!("vertex_ai/{model}");
    [prefixed.as_str(), model]
        .into_iter()
        .find_map(|key| {
            let canonical = catalog.lookup.get(key)?;
            catalog.entries.get(canonical.as_ref())
        })
        .is_some_and(|entry| entry.fields.capabilities.media.uses_embed_content == Some(true))
}
