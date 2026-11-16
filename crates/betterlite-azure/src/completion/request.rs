use std::collections::BTreeMap;

use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use secrecy::{ExposeSecret as _, SecretString};

pub const CLOUDFLARE_MARKER: &str = "gateway.ai.cloudflare.com";
pub const DEFAULT_MAX_RETRIES: i64 = 2;
pub const MAX_RETRIES_KEY: &str = "max_retries";
pub const STREAM_KEY: &str = "stream";

// optional_params.pop("max_retries", 2) keeps the raw value: only the sync guard inspects its type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retries(Json);

impl Retries {
    #[must_use]
    pub fn pop(data: &mut Extra) -> Self {
        match data.shift_remove(MAX_RETRIES_KEY) {
            Some(value) => Self(value),
            None => Self(Json::Number(serde_json::Number::from(DEFAULT_MAX_RETRIES))),
        }
    }

    // isinstance(value, int) accepts a bool, which Python's JSON decoder builds for true and false
    #[must_use]
    pub fn is_int(&self) -> bool {
        match &self.0 {
            Json::Bool(_) => true,
            Json::Number(number) => number.as_i64().is_some() || number.as_u64().is_some(),
            Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
        }
    }

    #[must_use]
    pub fn value(&self) -> &Json {
        &self.0
    }
}

pub fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|value| value != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// optional_params.get("stream", False)
pub fn optional_stream(optional_params: &Extra) -> bool {
    optional_params.get(STREAM_KEY).is_some_and(truthy)
}

// optional_params["stream"] is True: the sync selector demands the literal boolean
pub fn literal_true_stream(optional_params: &Extra) -> bool {
    matches!(optional_params.get(STREAM_KEY), Some(Json::Bool(true)))
}

pub fn is_cloudflare(api_base: Option<&str>) -> bool {
    api_base.is_some_and(|base| base.contains(CLOUDFLARE_MARKER))
}

// {"model": model, "prompt": prompt, **optional_params}: the overlay replaces model and prompt too
pub fn build_data(
    model: Option<&str>,
    prompt: &str,
    optional_params: &Extra,
    cloudflare: bool,
) -> Extra {
    let mut data = Extra::new();
    let model = match (cloudflare, model) {
        (false, Some(name)) => Json::Text(name.to_owned()),
        (false, None) | (true, _) => Json::Null,
    };
    data.insert("model".to_owned(), model);
    data.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    for (key, value) in optional_params {
        data.insert(key.clone(), value.clone());
    }
    data
}

#[must_use]
pub fn validate_environment(
    api_key: Option<&SecretString>,
    azure_ad_token: Option<&SecretString>,
) -> SecretHeaders {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    match (api_key, azure_ad_token) {
        (Some(key), _discarded) => {
            headers.insert("api-key".to_owned(), key.expose_secret().to_owned());
        }
        (None, Some(token)) => {
            headers.insert(
                "Authorization".to_owned(),
                format!("Bearer {}", token.expose_secret()),
            );
        }
        (None, None) => {}
    }
    SecretHeaders(headers)
}
