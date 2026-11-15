use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::ovhcloud_chat::DEFAULT_API_BASE;

use super::EMBEDDING_ENDPOINT;

// OVHCloudEmbeddingConfig.get_supported_openai_params returns an empty list, so the request mapping copies nothing
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    format!(
        "{}/{EMBEDDING_ENDPOINT}",
        api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'))
    )
}

// the key is the explicit api_key else the caller-read OVHCLOUD_API_KEY, and a missing one still writes Bearer None
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env_api_key: Option<&SecretString>,
) -> Headers {
    let mut defaults = Headers::new();
    let key = api_key.or(env_api_key);
    defaults.insert(
        String::from("Authorization"),
        key.map_or_else(
            || String::from("Bearer None"),
            |secret| format!("Bearer {}", secret.expose_secret()),
        ),
    );
    defaults.insert(String::from("accept"), String::from("application/json"));
    defaults.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    defaults.extend(headers.clone());
    defaults
}

// the supported list is empty, so the mapping keeps the accumulator it was handed and adds nothing to it
#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    optional_params
}

// options sit over input and model, so an option with either name wins exactly as the dict merge does
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Json, optional_params: &Object) -> Object {
    let mut body = Object::new();
    body.insert(String::from("input"), input.clone());
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    for (name, value) in optional_params {
        body.insert(name.clone(), value.clone());
    }
    body
}
