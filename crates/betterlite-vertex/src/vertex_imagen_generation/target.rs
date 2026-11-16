use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::ImageIdentitySettings;
use crate::vertex_image_identity::target::{ImageTarget, complete_url};

// get_complete_url: a truthy custom base is the whole URL, and predict only names the path complete_url builds
pub fn predict_url(
    settings: &ImageIdentitySettings,
    target: &ImageTarget<'_>,
) -> Result<String, ImageIdentityError> {
    if let Some(api_base) = target.api_base.filter(|base| !base.is_empty()) {
        return Ok(api_base.trim_end_matches('/').to_owned());
    }
    // complete_url builds the Gemini model URL, and predict names the same model path with another method
    let url = complete_url(settings, target)?;
    if let Some(publisher) = url.strip_suffix(":generateContent") {
        return Ok(format!("{publisher}:predict"));
    }
    Ok(url)
}
