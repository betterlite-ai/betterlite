use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;

#[derive(Debug, thiserror::Error)]
pub enum SamplingError {
    #[error("standard_callback_dynamic_params must be an object")]
    ParamsType,
    #[error("standard_callback_dynamic_params.langsmith_sampling_rate must be a number")]
    RateType,
}

// LangsmithLogger._get_sampling_rate_to_use_for_request; the request rate wins over the logger's default.
pub fn sampling_rate_for_request(
    default_rate: f64,
    dynamic_params: Option<&StandardCallbackDynamicParams>,
) -> Result<f64, SamplingError> {
    let Some(dynamic) = dynamic_params else {
        return Ok(default_rate);
    };
    let Json::Object(fields) = dynamic else {
        return Err(SamplingError::ParamsType);
    };
    match fields.get("langsmith_sampling_rate") {
        None | Some(Json::Null) => Ok(default_rate),
        Some(Json::Number(number)) => number.as_f64().ok_or(SamplingError::RateType),
        // float() reads a team callback var's decimal text, so "0.5" from callback_vars is a rate too
        Some(Json::Text(text)) => text
            .trim()
            .parse::<f64>()
            .map_err(|_source| SamplingError::RateType),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => Err(SamplingError::RateType),
    }
}
