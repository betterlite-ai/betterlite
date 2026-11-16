use betterlite_openai::image_generation::family::ImageFamily;

use super::error::FoundryImageError;

// LiteLLM inherits the GPT image list and appends the five it then drops, so the order is part of the reported allowlist
pub const FLUX2_OPENAI_PARAMS: [&str; 16] = [
    "n",
    "size",
    "output_format",
    "seed",
    "safety_tolerance",
    "aspect_ratio",
    "width",
    "height",
    "num_images",
    "guidance",
    "steps",
    "background",
    "moderation",
    "output_compression",
    "quality",
    "user",
];
pub const FLUX2_DROPPED_OPENAI_PARAMS: [&str; 5] = [
    "background",
    "moderation",
    "output_compression",
    "quality",
    "user",
];
const DEFAULT_API_VERSION: &str = "preview";

#[must_use]
pub fn is_flux2_model(model: &str) -> bool {
    let normalized = model.to_lowercase().replace(['.', '_'], "-");
    for marker in ["flux-2", "flux2"] {
        if normalized.contains(marker) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn flux2_provider_model_path(model: &str) -> &'static str {
    if model
        .to_lowercase()
        .replace(['.', '_'], "-")
        .contains("flux-2-flex")
    {
        "flux-2-flex"
    } else {
        "flux-2-pro"
    }
}

pub fn flux2_image_generation_url(
    api_base: Option<&str>,
    model: &str,
    api_version: Option<&str>,
) -> Result<String, FoundryImageError> {
    let base = api_base.ok_or(FoundryImageError::ApiBaseRequired)?;
    let trimmed_base = base.trim_end_matches('/');
    let version = api_version
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or(DEFAULT_API_VERSION);
    if trimmed_base.contains("/providers/") {
        return Ok(if trimmed_base.contains('?') {
            trimmed_base.to_owned()
        } else {
            format!("{trimmed_base}?api-version={version}")
        });
    }
    let path = flux2_provider_model_path(model);
    Ok(format!(
        "{trimmed_base}/providers/blackforestlabs/v1/{path}?api-version={version}"
    ))
}

#[must_use]
pub fn supported_params(model: &str) -> &'static [&'static str] {
    if is_flux2_model(model) {
        &FLUX2_OPENAI_PARAMS
    } else {
        ImageFamily::GptImage.supported_params()
    }
}
