use betterlite_prompts::arguments::json::{Json, Object};

use super::capabilities::{ADAPTIVE_THINKING, ThinkingCapabilities};

pub(crate) struct TemperatureRule<'request> {
    pub(crate) model: &'request str,
    pub(crate) custom_llm_provider: &'request str,
    pub(crate) optional_params: &'request mut Object,
}

pub(crate) async fn drop_incompatible_temperature<Capabilities>(
    capabilities: &mut Capabilities,
    request: TemperatureRule<'_>,
) where
    Capabilities: ThinkingCapabilities + Send,
{
    if capabilities
        .supports(
            request.model,
            ADAPTIVE_THINKING,
            request.custom_llm_provider,
        )
        .await
    {
        return;
    }
    let params = request.optional_params;
    let temperature = params.get("temperature");
    if matches!(temperature, None | Some(Json::Null)) {
        return;
    }
    if matches!(temperature, Some(Json::Number(number)) if number.as_f64() == Some(1.0_f64)) {
        return;
    }
    let thinking_enabled = matches!(
        params.get("thinking"),
        Some(Json::Object(fields))
            if fields.get("type").and_then(Json::as_str) == Some("enabled")
    );
    let effort_enabled = matches!(
        params.get("output_config"),
        Some(Json::Object(fields))
            if fields
                .get("effort")
                .is_some_and(|effort| !matches!(effort, Json::Null))
    );
    if thinking_enabled || effort_enabled {
        params.shift_remove("temperature");
    }
}
