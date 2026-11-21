use super::error::HealthProbeError;
use base64::Engine as _;
pub const IMAGE_EDIT_HEALTH_CHECK_PROMPT: &str = "Add a small yellow star in the top right corner of this simple drawing of a blue circle on a white background";
// LiteLLM's TEST_IMAGE_BASE64, copied byte for byte: a 512 by 512 PNG of a blue circle on white
const TEST_IMAGE_BASE64: &str = include_str!("health_check_image.b64");
pub(crate) fn health_check_image() -> Result<Vec<u8>, HealthProbeError> {
    let text = TEST_IMAGE_BASE64.trim();
    let image = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|source| HealthProbeError::HealthImage { source })?;
    Ok(image)
}
