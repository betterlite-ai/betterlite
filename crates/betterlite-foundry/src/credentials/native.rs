use betterlite_context::chat_config_selection::foundry::{AzureAiRoute, get_azure_ai_route};
use betterlite_openai::reasoning_models::{is_model_gpt_5_6_plus_model, is_model_gpt_6_plus_model};
use betterlite_prompts::arguments::json::Json;

use super::host::is_azure_openai_v1_host;
use super::model_info::ModelInfoAccessors;

#[must_use]
pub fn supports_native_responses(
    model: Option<&str>,
    api_base: Option<&str>,
    accessors: ModelInfoAccessors<'_>,
) -> bool {
    if let Some(resolved_base) = accessors.api_base(api_base)
        && !is_azure_openai_v1_host(Some(&resolved_base))
    {
        return false;
    }
    let Some(model) = model else {
        return true;
    };
    if model.to_lowercase().contains("claude") {
        return false;
    }
    get_azure_ai_route(model) == AzureAiRoute::Default
}

#[must_use]
pub fn foundry_rejects_tools(model: &str, reasoning_effort: Option<&Json>) -> bool {
    match reasoning_effort {
        None | Some(Json::Null) => is_model_gpt_6_plus_model(model),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            is_model_gpt_5_6_plus_model(model)
        }
    }
}
