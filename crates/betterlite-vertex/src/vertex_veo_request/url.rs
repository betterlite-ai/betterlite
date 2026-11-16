use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::api::vertex_location::{validate_vertex_location, validate_vertex_project};

use crate::credential_sources::getters::VertexParams;
use crate::model_urls::base_url::get_vertex_base_url;
use crate::model_urls::error::VertexUrlError;

use super::error::VeoRequestError;

pub const DEFAULT_LOCATION: &str = "us-central1";

// get_complete_url: the configured project is required; header authentication never fills it in
pub fn complete_url(
    model: &str,
    api_base: Option<&str>,
    params: VertexParams<'_>,
    settings: &VertexSettings,
) -> Result<String, VeoRequestError> {
    let project = params
        .safe_get_vertex_ai_project(settings)
        .filter(|project| !project.is_empty())
        .ok_or(VeoRequestError::MissingProject)?;
    let project = validate_vertex_project(project).map_err(VeoRequestError::Project)?;
    let location = params
        .safe_get_vertex_ai_location(settings)
        .filter(|location| !location.is_empty())
        .unwrap_or(DEFAULT_LOCATION);
    let location = validate_vertex_location(Some(location))
        .map_err(VertexUrlError::from)
        .map_err(VeoRequestError::Location)?;
    let base_url = match api_base.filter(|base| !base.is_empty()) {
        Some(base) => base.trim_end_matches('/').to_owned(),
        None => get_vertex_base_url(Some(location)).map_err(VeoRequestError::Location)?,
    };
    let model_name = model.replace("vertex_ai/", "");
    Ok(format!(
        "{base_url}/v1/projects/{project}/locations/{location}/publishers/google/models/{model_name}"
    ))
}
