use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::dynamic_limits::quota::AvailableUsage;

pub const MODEL_GROUP_HEADER: &str = "x-litellm-model_group";
pub const PROJECT_TOKENS_HEADER: &str = "x-ratelimit-remaining-litellm-project-tokens";
pub const PROJECT_REQUESTS_HEADER: &str = "x-ratelimit-remaining-litellm-project-requests";
pub const MODEL_TOKENS_HEADER: &str = "x-ratelimit-remaining-model-tokens";
pub const MODEL_REQUESTS_HEADER: &str = "x-ratelimit-remaining-model-requests";
pub const ACTIVE_PROJECTS_HEADER: &str = "x-ratelimit-current-active-projects";

// async_post_call_success_hook's block: the model group as text and each count as a JSON number, or null when it is unknown
pub fn write_usage_headers(
    headers: &mut AdditionalHeaders,
    model_group: &str,
    usage: &AvailableUsage,
) {
    headers.insert(
        String::from(MODEL_GROUP_HEADER),
        Json::Text(model_group.to_owned()),
    );
    for (name, value) in [
        (PROJECT_TOKENS_HEADER, usage.available_tpm),
        (PROJECT_REQUESTS_HEADER, usage.available_rpm),
        (MODEL_TOKENS_HEADER, usage.model_tpm),
        (MODEL_REQUESTS_HEADER, usage.model_rpm),
        (ACTIVE_PROJECTS_HEADER, usage.active_projects),
    ] {
        // a value LiteLLM holds as a Python int or None, which the response carries as a JSON number or null
        headers.insert(
            String::from(name),
            value.map_or(Json::Null, |number| Json::Number(Number::from(number))),
        );
    }
}
