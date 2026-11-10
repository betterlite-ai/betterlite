use betterlite_models::provider_defaults_a::url::{parse, unparse};

use super::error::HostModelIdentityError;

// ClarifaiConfig.get_supported_openai_params: a fixed list, not the OpenAI family's catalogue
pub const SUPPORTED_OPENAI_PARAMS: [&str; 11] = [
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "stream",
    "temperature",
    "top_p",
    "tool_choice",
    "tools",
    "presence_penalty",
    "frequency_penalty",
    "stream_options",
];

// ClarifaiConfig.get_base_model, whose None means the caller keeps the model it sent
pub fn clarifai_base_model(model: Option<&str>) -> Result<Option<String>, HostModelIdentityError> {
    match model.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => {
            let parts: Vec<&str> = text.split('.').collect();
            let [user_id, app_id, model_id] = parts.as_slice() else {
                return Err(HostModelIdentityError::ModelName {
                    model: text.to_owned(),
                });
            };
            Ok(Some(format!(
                "https://clarifai.com/{user_id}/{app_id}/models/{model_id}"
            )))
        }
    }
}

// ClarifaiConfig.transform_request hands the parent `get_base_model(model) or model`
pub fn clarifai_request_model(
    model: Option<&str>,
) -> Result<Option<String>, HostModelIdentityError> {
    let converted = clarifai_base_model(model)?;
    if converted.is_none() {
        return Ok(model.map(str::to_owned));
    }
    Ok(converted)
}

const LLMGW_PATH: &str = "/genai/llmgw/chat/completions";
const LLMGW_SUFFIX: &str = "/api/v2/genai/llmgw/chat/completions";

// DataRobotConfig._resolve_api_base edits only the path of the endpoint the caller configured
pub fn datarobot_api_base(api_base: &str) -> Result<String, HostModelIdentityError> {
    let mut parts = parse(api_base).map_err(|source| HostModelIdentityError::ApiBase {
        api_base: api_base.to_owned(),
        source,
    })?;
    let path = &parts.path;
    let mut resolved = if path.is_empty() || path == "/" {
        format!("{path}{LLMGW_SUFFIX}")
    } else if path.contains("api/v2")
        && !path.contains("api/v2/deployments")
        && !path.contains(LLMGW_PATH)
    {
        format!("{path}{LLMGW_PATH}")
    } else {
        path.clone()
    };
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    // one pass, so a longer run of slashes keeps the pairs a single pass leaves behind
    parts.path = resolved.replace("//", "/");
    Ok(unparse(&parts))
}
