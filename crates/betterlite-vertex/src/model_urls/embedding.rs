use betterlite_models::catalog::ModelCatalog;
use betterlite_models::model_catalog_keys::strip_stable_vertex_version;

use super::base_url::validate_vertex_location;
use super::error::VertexUrlError;
use super::model_name::get_vertex_base_model_name;
use super::path::{VertexPath, model_path};
use super::{ApiVersion, Endpoint, VertexUrl};

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
        Endpoint::EmbedContent
    } else {
        Endpoint::Predict
    };
    let path = VertexPath {
        model,
        project,
        location,
    };
    Ok(VertexUrl {
        url: model_path(&path, input.api_version, ApiVersion::V1, endpoint),
        endpoint,
    })
}

// get_model_info reads vertex_ai/<model>, <model>, the split name, then both with the version suffix removed
fn uses_embed_content(catalog: &ModelCatalog, model: &str) -> bool {
    let split = model.strip_prefix("vertex_ai/").unwrap_or(model);
    let stripped = strip_stable_vertex_version(split);
    let prefixed = format!("vertex_ai/{model}");
    let stripped_prefixed = format!("vertex_ai/{stripped}");
    [
        prefixed.as_str(),
        model,
        split,
        stripped_prefixed.as_str(),
        stripped.as_str(),
    ]
    .into_iter()
    .find_map(|key| {
        let canonical = catalog.lookup.get(key)?;
        catalog.entries.get(canonical.as_ref())
    })
    .is_some_and(|entry| entry.fields.capabilities.media.uses_embed_content == Some(true))
}
