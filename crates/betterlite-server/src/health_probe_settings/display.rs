use betterlite_models::model_registration::JsonObject;

// Provider routing fields only a proxy admin sees; the endpoint gates them for other callers
pub const ADMIN_ONLY_DISPLAY_PARAMS: [&str; 3] =
    ["api_base", "api_version", "aws_bedrock_runtime_endpoint"];

pub const MINIMAL_DISPLAY_PARAMS: [&str; 2] = ["model", "mode_error"];

pub const HEALTH_DISPLAY_PARAMS: [&str; 20] = [
    "model",
    "mode_error",
    "api_base",
    "api_version",
    "aws_bedrock_runtime_endpoint",
    "custom_llm_provider",
    "mode",
    "base_model",
    "aws_region_name",
    "region_name",
    "watsonx_region_name",
    "vertex_project",
    "vertex_location",
    "tpm",
    "rpm",
    "error",
    "raw_request_typed_dict",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-remaining-tokens",
    "x-ms-region",
];

#[must_use]
pub fn clean_endpoint_data(endpoint_data: &JsonObject, details: bool) -> JsonObject {
    let displayed: &[&str] = if details {
        &HEALTH_DISPLAY_PARAMS
    } else {
        &MINIMAL_DISPLAY_PARAMS
    };
    endpoint_data
        .iter()
        .filter(|(key, _value)| displayed.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
