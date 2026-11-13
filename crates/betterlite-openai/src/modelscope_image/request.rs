use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::openai_base_config::{Headers, Object};

use super::error::ModelScopeImageError;

pub const DEFAULT_BASE_URL: &str = "https://api-inference.modelscope.cn/v1";
const ENDPOINT_PATH: &str = "/images/generations";

#[must_use]
pub fn get_complete_url(api_base: Option<&str>, environment_base: Option<&str>) -> String {
    format!(
        "{}{ENDPOINT_PATH}",
        api_base
            .or(environment_base)
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
    )
}

pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment_api_key: Option<&SecretString>,
) -> Result<Headers, ModelScopeImageError> {
    let key = api_key
        .or(environment_api_key)
        .ok_or(ModelScopeImageError::MissingApiKey)?;
    let mut merged = headers.clone();
    merged.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    merged.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    Ok(merged)
}

// ModelScope takes the OpenAI body: model and prompt, then every option under a public name
#[must_use]
pub fn image_request_body(model: &str, prompt: &str, optional_params: &Object) -> Object {
    let mut body = Object::from_iter([
        (String::from("model"), Value::from(model)),
        (String::from("prompt"), Value::from(prompt)),
    ]);
    for (name, value) in optional_params {
        if !name.starts_with('_') {
            body.insert(name.clone(), value.clone());
        }
    }
    body
}
