use crate::model_urls::base_url::get_vertex_base_url;
use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::ImageIdentitySettings;
use crate::vertex_image_identity::target::ImageTarget;

use super::error::ImagenEditError;

// get_complete_url: project and location are required even behind a custom base, which replaces only the host
pub fn predict_url(
    settings: &ImageIdentitySettings,
    target: &ImageTarget<'_>,
) -> Result<String, ImagenEditError> {
    let resolved = settings.resolve(target.params);
    let project = resolved.project.ok_or(ImageIdentityError::MissingTarget)?;
    let location = resolved.location.ok_or(ImageIdentityError::MissingTarget)?;
    let model_name = if target.model.starts_with("vertex_ai/") {
        target.model.replace("vertex_ai/", "")
    } else {
        target.model.to_owned()
    };
    let base_url = match target.api_base.filter(|base| !base.is_empty()) {
        Some(base) => base.trim_end_matches('/').to_owned(),
        None => get_vertex_base_url(Some(location)).map_err(ImageIdentityError::Location)?,
    };
    Ok(format!(
        "{base_url}/v1/projects/{project}/locations/{location}/publishers/google/models/{model_name}:predict"
    ))
}
