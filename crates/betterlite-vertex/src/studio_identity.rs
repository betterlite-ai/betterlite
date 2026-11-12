pub mod decode;
pub mod error;
pub mod models;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use secrecy::SecretString;

pub const API_VERSION: &str = "v1beta";

pub type Headers = BTreeMap<String, SecretString>;

#[derive(Debug, Clone)]
pub enum AuthValue {
    Key(SecretString),
    Headers(BTreeMap<String, SecretString>),
}

#[derive(Debug, Clone, Default)]
pub struct NativeParams {
    pub api_key: Option<AuthValue>,
    pub gemini_api_key: Option<AuthValue>,
}

#[must_use]
pub fn process_model_names(names: &[String]) -> Vec<String> {
    let mut litellm_model_names = Vec::with_capacity(names.len());
    for name in names {
        litellm_model_names.push(format!("gemini/{}", name.replace("models/", "")));
    }
    litellm_model_names
}

#[must_use]
pub fn native_api_key(
    explicit: Option<&AuthValue>,
    params: &NativeParams,
    settings: &Settings,
) -> Option<AuthValue> {
    let from_params = explicit
        .or(params.api_key.as_ref())
        .or(params.gemini_api_key.as_ref());
    if let Some(auth) = from_params {
        return Some(auth.clone());
    }
    if let Some(key) = settings.environment_key.as_ref() {
        return Some(AuthValue::Key(key.clone()));
    }
    settings
        .generic_api_key
        .as_ref()
        .map(|key| AuthValue::Key(key.clone()))
}

#[must_use]
pub fn native_headers(
    explicit: Option<&AuthValue>,
    params: &NativeParams,
    settings: &Settings,
    caller: &BTreeMap<String, String>,
) -> Headers {
    let mut headers = Headers::from([(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    )]);
    match native_api_key(explicit, params, settings) {
        Some(AuthValue::Key(key)) => {
            headers.insert("x-goog-api-key".to_owned(), key);
        }
        Some(AuthValue::Headers(auth)) => headers.extend(auth),
        None => {}
    }
    for (name, value) in caller {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers
}
