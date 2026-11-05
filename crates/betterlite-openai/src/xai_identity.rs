mod error;
mod tests;

use betterlite_config::sections::xai_identity::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::openai_base_config::Headers;

pub use error::XaiIdentityError;

const DEFAULT_API_BASE: &str = "https://api.x.ai";
const MODEL_LIST_LIMIT: usize = 4 * 1024 * 1024;

// chat and the other endpoint families disagree on where litellm.api_key sits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrder {
    Chat,
    GenericBeforeEnvironment,
}

#[derive(Debug)]
pub struct ModelsRequest {
    pub url: String,
    pub authorization: SecretString,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[must_use]
pub fn get_api_key(
    api_key: Option<&SecretString>,
    order: KeyOrder,
    settings: &Settings,
) -> Option<SecretString> {
    let generic = match order {
        KeyOrder::Chat => None,
        KeyOrder::GenericBeforeEnvironment => settings.generic_api_key.as_ref(),
    };
    api_key
        .or(settings.xai_key.as_ref())
        .or(generic)
        .or(settings.environment_key.as_ref())
        .cloned()
}

#[must_use]
pub fn get_api_base(api_base: Option<&str>, settings: &Settings) -> String {
    api_base
        .or(settings.api_base.as_deref())
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned()
}

#[must_use]
pub fn validate_environment(mut headers: Headers, api_key: Option<&SecretString>) -> Headers {
    if let Some(key) = api_key {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    if !headers.contains_key("content-type") && !headers.contains_key("Content-Type") {
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    }
    headers
}

pub fn models_request(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    settings: &Settings,
) -> Result<ModelsRequest, XaiIdentityError> {
    let base = get_api_base(api_base, settings);
    let key = get_api_key(api_key, KeyOrder::Chat, settings)
        .ok_or(XaiIdentityError::MissingCredentials)?;
    Ok(ModelsRequest {
        url: format!("{base}/v1/models"),
        authorization: SecretString::from(format!("Bearer {}", key.expose_secret())),
    })
}

pub fn decode_xai_models(status: u16, bytes: &[u8]) -> Result<Vec<String>, XaiIdentityError> {
    if bytes.len() > MODEL_LIST_LIMIT {
        return Err(XaiIdentityError::ResponseTooLarge {
            limit: MODEL_LIST_LIMIT,
        });
    }
    if !(200..300).contains(&status) {
        return Err(XaiIdentityError::Status {
            status,
            body: String::from_utf8_lossy(bytes).into_owned(),
        });
    }
    let list: ModelList = serde_json::from_slice(bytes)?;
    Ok(list
        .data
        .into_iter()
        .map(|model| format!("xai/{}", model.id))
        .collect())
}
