use betterlite_models::decisions::Json;

use super::OracleLimits;
use super::error::SdkHttpError;

// Every call that reaches a provider gives its request the same bounded timeout
pub(crate) fn validate_timeout(seconds: u64, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    if seconds == 0 || seconds > limits.max_execution.as_secs() {
        return Err(SdkHttpError::Invalid(
            "timeout_seconds must be between 1 and the execution limit".to_owned(),
        ));
    }
    Ok(())
}

// A case may only seed the held buffers with the chunk objects the driver deserializes
pub(crate) fn validate_held(
    held: Option<&Json>,
    stop_reason: Option<&Json>,
) -> Result<(), SdkHttpError> {
    for seed in [held, stop_reason].into_iter().flatten() {
        if !matches!(seed, Json::Object(_)) {
            return Err(SdkHttpError::Invalid(
                "the initial held chunks must be objects".to_owned(),
            ));
        }
    }
    Ok(())
}
