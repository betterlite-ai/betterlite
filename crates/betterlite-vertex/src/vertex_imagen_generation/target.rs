use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::ImageIdentitySettings;
use crate::vertex_image_identity::target::{ImageTarget, complete_url};

// complete_url builds the Gemini model URL, and predict names the same model path with another method
pub fn predict_url(
    settings: &ImageIdentitySettings,
    target: &ImageTarget<'_>,
) -> Result<String, ImageIdentityError> {
    let url = complete_url(settings, target)?;
    match url.strip_suffix(":generateContent") {
        Some(publisher) => Ok(format!("{publisher}:predict")),
        // a truthy custom base is the whole URL and names no endpoint to swap
        None => Ok(url),
    }
}
