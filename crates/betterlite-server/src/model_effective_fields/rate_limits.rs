use betterlite_config::sections::model_effective_fields::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::ModelFieldError;

// a zero or negative limit would keep the deployment out of routing, so it counts as missing
pub fn check_rate_limits(params: &JsonObject, settings: &Settings) -> Result<(), ModelFieldError> {
    if !settings.enforce_rpm_tpm_on_model_add {
        return Ok(());
    }
    let missing: Vec<&'static str> = ["rpm", "tpm"]
        .into_iter()
        .filter(|field| {
            !matches!(
                params.get(*field),
                Some(Json::Number(limit)) if limit.as_u64().is_some_and(|value| value > 0_u64)
            )
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(ModelFieldError::RateLimitsMissing { missing })
}
