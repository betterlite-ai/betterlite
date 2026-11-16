use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use serde_json::Number;

use crate::databricks_auth::Headers;
use crate::databricks_auth::api_base::get_api_base;
use crate::databricks_auth::endpoint::EndpointType;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::DatabricksError;
use crate::databricks_auth::oauth::OauthTokenSource;
use crate::databricks_auth::validate::{
    ValidationRequest, validate_environment as auth_validate_environment,
};
use crate::databricks_chat::error::DatabricksChatError;

pub mod params;
pub mod tools;

mod tests;

pub(crate) const CLAUDE: &str = "claude";
pub(crate) const PROVIDER: &str = "databricks";

pub const SUPPORTED_OPENAI_PARAMS: [&str; 12] = [
    "stream",
    "stop",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "n",
    "response_format",
    "tools",
    "tool_choice",
    "reasoning_effort",
    "thinking",
];

pub struct ProviderField {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_description: &'static str,
    pub field_value: &'static str,
}

#[must_use]
pub const fn get_required_params() -> [ProviderField; 2] {
    [
        ProviderField {
            field_name: "api_key",
            field_type: "string",
            field_description: "Your Databricks API Key.",
            field_value: "dapi...",
        },
        ProviderField {
            field_name: "api_base",
            field_type: "string",
            field_description: "Your Databricks API Base.",
            field_value: "https://adb-..",
        },
    ]
}

// OpenAIGPTConfig.get_config, completed by AnthropicConfig with its default chat max tokens
#[must_use]
pub fn get_config(default_chat_max_tokens: i64) -> Object {
    Object::from_iter([(
        String::from("max_tokens"),
        Json::Number(Number::from(default_chat_max_tokens)),
    )])
}

// get_complete_url: two dots after the provider prefix turn the AI gateway on and trim the base
pub fn get_complete_url(
    api_base: Option<&str>,
    model: &str,
) -> Result<String, DatabricksChatError> {
    let gateway_model = model.strip_prefix("databricks/").unwrap_or(model);
    let use_ai_gateway = gateway_model.matches('.').count() >= 2;
    let base = get_api_base(api_base, use_ai_gateway)?;
    let url_base = if use_ai_gateway {
        base.trim_end_matches('/')
    } else {
        base.as_str()
    };
    Ok(format!("{url_base}/chat/completions"))
}

pub struct EnvironmentRequest<'value> {
    pub api_key: Option<&'value SecretString>,
    pub api_base: Option<&'value str>,
    pub headers: Option<Headers>,
    pub litellm_user_agent: Option<&'value str>,
    pub env_user_agent: Option<&'value str>,
}

// validate_environment: the user agent chain pops optional_params first, then caller values
pub async fn validate_environment(
    request: EnvironmentRequest<'_>,
    optional_params: &mut Object,
    env: &DatabricksEnv,
    tokens: &dyn OauthTokenSource,
) -> Result<Headers, DatabricksChatError> {
    let from_params = pop_user_agent_parameter(optional_params)?;
    let custom_user_agent = from_params
        .or_else(|| non_empty(request.litellm_user_agent))
        .or_else(|| non_empty(request.env_user_agent))
        .or_else(|| non_empty(env.user_agent_override.as_deref()));
    let validation = ValidationRequest {
        api_key: request.api_key,
        api_base: request.api_base,
        endpoint: EndpointType::ChatCompletions,
        custom_endpoint: false,
        headers: request.headers,
        custom_user_agent: custom_user_agent.as_deref(),
    };
    let (_url, mut headers) = auth_validate_environment(validation, env, tokens).await?;
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    Ok(headers)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        None | Some(_) => None,
    }
}

// A user agent from the optional parameters is popped even when it is null; empty falls through
pub fn pop_user_agent(
    optional_params: &mut Object,
    name: &str,
) -> Result<Option<String>, DatabricksChatError> {
    let Some(value) = optional_params.shift_remove(name) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text).filter(|agent| !agent.is_empty())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(DatabricksChatError::Databricks(DatabricksError {
                status_code: 400,
                message: format!("the {name} optional parameter must be a string"),
                headers: None,
            }))
        }
    }
}

pub fn pop_user_agent_parameter(
    optional_params: &mut Object,
) -> Result<Option<String>, DatabricksChatError> {
    if let Some(user_agent) = pop_user_agent(optional_params, "user_agent")? {
        return Ok(Some(user_agent));
    }
    // the Python "or" only pops databricks_user_agent when user_agent is absent or empty
    pop_user_agent(optional_params, "databricks_user_agent")
}

// Databricks does not support response_format while streaming, so the caller fakes the stream
#[must_use]
pub fn should_fake_stream(optional_params: &Object) -> bool {
    optional_params
        .get("response_format")
        .is_some_and(|response_format| !matches!(response_format, Json::Null))
}
