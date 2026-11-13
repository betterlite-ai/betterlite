use betterlite_prompts::arguments::json::Json;

use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::error::GeminiOptionsError;

// LiteLLM keeps these three for Gemini 3 but warns once, on the first one it maps
pub const SAMPLING_PARAMS: [&str; 3] = ["temperature", "top_p", "top_k"];

// One warning per request whatever combination arrives, and the value is still mapped
pub fn check_sampling(
    model: &str,
    param: &str,
    value: &Json,
    warned: &mut bool,
) -> Result<(), GeminiOptionsError> {
    let Json::Number(number) = value else {
        return Err(GeminiOptionsError::FieldType {
            param: param.to_owned(),
            expected: "a number",
        });
    };
    let Some(parsed) = number.as_f64() else {
        return Err(GeminiOptionsError::NonFinite {
            field: param.to_owned(),
        });
    };
    if !is_gemini_3_or_newer(model) {
        return Ok(());
    }
    if param == "temperature" && parsed < 1.0_f64 {
        tracing::info!(
            model,
            "temperature below 1.0 for Gemini 3 can cause loops and weaker reasoning; 1.0 is recommended"
        );
    }
    if !*warned {
        tracing::warn!(
            model,
            "temperature, top_p and top_k still work for Gemini 3 but are planned for removal; put sampling guidance in the system instructions instead"
        );
        *warned = true;
    }
    Ok(())
}
