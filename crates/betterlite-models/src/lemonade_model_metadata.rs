use std::error::Error;
use std::future::Future;
use std::time::Duration;

use indexmap::IndexMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

use crate::decisions::Json;
use crate::fetch_policy::response::FetchResponse;
use crate::metadata::identity::Mode;

mod tests;

pub const DEFAULT_API_BASE: &str = "http://localhost:8000/api/v1";
pub const DEFAULT_API_KEY: &str = "lemonade";
pub const LITELLM_PROVIDER: &str = "lemonade";

// Python's urllib.parse.quote always leaves the letters, digits and "_.-~" alone
const QUOTE_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

const ZERO_PRICE: Decimal = Decimal::from_parts(0, 0, 0, false, 1);

#[derive(Clone, Debug, Default)]
pub struct LemonadeCredentials {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub lemonade_key: Option<SecretString>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeploymentCredentials<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
}

#[derive(Clone, Debug)]
pub struct ResolvedOrigin {
    pub api_base: String,
    pub api_key: SecretString,
}

#[derive(Clone, Debug)]
pub struct ModelInfoRequest {
    pub url: String,
    pub headers: Vec<(&'static str, SecretString)>,
    pub timeout: Duration,
}

pub trait ModelInfoClient: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum ModelInfoReplyError {
    #[error("the Lemonade model info reply is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Lemonade model info reply is not a JSON object")]
    NotObject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LemonadeModelInfo {
    pub key: String,
    pub litellm_provider: String,
    pub mode: Mode,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub input_cost_per_token: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub output_cost_per_token: Decimal,
    pub max_tokens: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_entry: Option<IndexMap<String, Json>>,
}

#[must_use]
pub fn resolve_origin(
    settings: &LemonadeCredentials,
    deployment: DeploymentCredentials<'_>,
) -> ResolvedOrigin {
    let api_base = deployment
        .api_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            settings
                .api_base
                .as_ref()
                .filter(|base| !base.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let explicit_key = deployment
        .api_key
        .filter(|key| !key.expose_secret().is_empty());
    let ambient_key = settings
        .lemonade_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            settings
                .api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        });
    let api_key = if deployment.api_base.is_none() || explicit_key.is_some() {
        explicit_key
            .or(ambient_key)
            .cloned()
            .unwrap_or_else(|| SecretString::from(DEFAULT_API_KEY))
    } else {
        SecretString::from(DEFAULT_API_KEY)
    };
    ResolvedOrigin { api_base, api_key }
}

#[must_use]
pub fn auth_header(api_key: &SecretString) -> Option<(&'static str, SecretString)> {
    let key = api_key.expose_secret();
    if key == DEFAULT_API_KEY {
        return None;
    }
    let bearer = format!("Bearer {key}");
    Some(("Authorization", SecretString::from(bearer)))
}

#[must_use]
pub fn positive_int(value: &Json) -> Option<u64> {
    match value {
        Json::Number(number) => number.as_u64().filter(|parsed| *parsed > 0),
        Json::Text(text) => int_from_text(text).filter(|parsed| *parsed > 0),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn int_from_text(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    let unsigned = trimmed.strip_prefix('+').unwrap_or(trimmed);
    // Python's int() accepts one underscore between digits, such as "1_000"
    let mut value: u64 = 0;
    let mut saw_digit = false;
    let mut after_underscore = true;
    for character in unsigned.chars() {
        if let Some(digit) = character.to_digit(10) {
            value = value.checked_mul(10)?.checked_add(u64::from(digit))?;
            saw_digit = true;
            after_underscore = false;
        } else if character == '_' && !after_underscore {
            after_underscore = true;
        } else {
            return None;
        }
    }
    (saw_digit && !after_underscore).then_some(value)
}

const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn provider_specific_entry(model_info: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut entry = match model_info.get("provider_specific_entry") {
        Some(Json::Object(fields)) => fields.clone(),
        _ => IndexMap::new(),
    };
    for key in ["recipe_options", "context_window", "max_context_window"] {
        if let Some(value) = model_info.get(key) {
            entry.insert(key.to_owned(), value.clone());
        }
    }
    entry
}

#[must_use]
pub fn context_window(model_info: &IndexMap<String, Json>) -> Option<u64> {
    let entry = provider_specific_entry(model_info);
    let recipe_options = entry.get("recipe_options").and_then(object);
    [
        recipe_options.and_then(|options| options.get("ctx_size")),
        model_info.get("max_input_tokens"),
        entry.get("context_window"),
        entry.get("max_context_window"),
    ]
    .into_iter()
    .flatten()
    .find_map(positive_int)
}

pub fn decode_lemonade_model_reply(
    bytes: &[u8],
) -> Result<IndexMap<String, Json>, ModelInfoReplyError> {
    let decoded: Json =
        serde_json::from_slice(bytes).map_err(|source| ModelInfoReplyError::Json { source })?;
    match decoded {
        Json::Object(model_info) => Ok(model_info),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ModelInfoReplyError::NotObject)
        }
    }
}

fn model_info_request(origin: &ResolvedOrigin, model: &str, timeout: Duration) -> ModelInfoRequest {
    let encoded = utf8_percent_encode(model, QUOTE_SAFE);
    ModelInfoRequest {
        url: format!("{}/models/{encoded}", origin.api_base),
        headers: auth_header(&origin.api_key).into_iter().collect(),
        timeout,
    }
}

fn project_model_info(
    mut info: LemonadeModelInfo,
    model_info: &IndexMap<String, Json>,
) -> LemonadeModelInfo {
    let provider_specific_entry = provider_specific_entry(model_info);
    info.max_input_tokens = context_window(model_info);
    info.max_output_tokens = model_info.get("max_output_tokens").and_then(positive_int);
    info.max_tokens = model_info
        .get("max_tokens")
        .and_then(positive_int)
        .or(info.max_output_tokens);
    info.provider_specific_entry =
        (!provider_specific_entry.is_empty()).then_some(provider_specific_entry);
    info
}

pub async fn get_model_info<Client>(
    client: &Client,
    settings: &LemonadeCredentials,
    model: &str,
    deployment: DeploymentCredentials<'_>,
    timeout: Duration,
) -> Result<LemonadeModelInfo, ModelInfoReplyError>
where
    Client: ModelInfoClient,
{
    let stripped = model.strip_prefix("lemonade/").unwrap_or(model);
    let origin = resolve_origin(settings, deployment);
    let request = model_info_request(&origin, stripped, timeout);
    let info = LemonadeModelInfo {
        key: format!("lemonade/{stripped}"),
        litellm_provider: LITELLM_PROVIDER.to_owned(),
        mode: Mode::Chat,
        input_cost_per_token: ZERO_PRICE,
        output_cost_per_token: ZERO_PRICE,
        max_tokens: None,
        max_input_tokens: None,
        max_output_tokens: None,
        provider_specific_entry: None,
    };
    let Ok(response) = client.get(&request).await else {
        tracing::debug!("LemonadeError: Could not get model info.");
        return Ok(info);
    };
    if !(200..300).contains(&response.status) {
        tracing::debug!("LemonadeError: Could not get model info.");
        return Ok(info);
    }
    let model_info = match decode_lemonade_model_reply(&response.body) {
        Ok(model_info) => model_info,
        Err(ModelInfoReplyError::Json { .. }) => {
            tracing::debug!("LemonadeError: Could not get model info.");
            return Ok(info);
        }
        Err(error @ ModelInfoReplyError::NotObject) => return Err(error),
    };
    Ok(project_model_info(info, &model_info))
}
