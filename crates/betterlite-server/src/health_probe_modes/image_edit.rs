use super::backend::HealthProbeBackend;
use super::calls::{HealthProbeReply, ImageEditCall, ensure_absent};
use super::error::HealthProbeError;
use super::image_data::{IMAGE_EDIT_HEALTH_CHECK_PROMPT, health_check_image};
use super::mode::HealthProbeMode;
use super::params::HealthProbeParams;
use betterlite_error::error::ErrorClass;
pub async fn probe_image_edit(
    params: &HealthProbeParams,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let parameters = params.call_parameters(false);
    ensure_absent(&parameters, "image", HealthProbeMode::ImageEdit)?;
    ensure_absent(&parameters, "prompt", HealthProbeMode::ImageEdit)?;
    let call = ImageEditCall {
        parameters,
        image: health_check_image()?,
        prompt: IMAGE_EDIT_HEALTH_CHECK_PROMPT,
        api_key: params.api_key.clone(),
        logging: params.litellm_logging_obj.clone(),
    };
    match backend.image_edit(call).await {
        Ok(reply) => Ok(reply),
        Err(error) if swallowed(&error) => Ok(HealthProbeReply::default()),
        Err(error) => Err(error),
    }
}
fn swallowed(error: &HealthProbeError) -> bool {
    let HealthProbeError::Backend { class, message, .. } = error else {
        return false;
    };
    is_bad_request(class)
        && (matches!(class, ErrorClass::ContentPolicyViolation(_))
            || message.contains("moderation_blocked"))
}
fn is_bad_request(class: &ErrorClass) -> bool {
    matches!(
        class,
        ErrorClass::BadRequest(_)
            | ErrorClass::ContextWindowExceeded(_)
            | ErrorClass::ContentPolicyViolation(_)
            | ErrorClass::UnsupportedParams(_)
    )
}
