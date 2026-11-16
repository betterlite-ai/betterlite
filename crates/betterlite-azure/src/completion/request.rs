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
pub struct Retries(pub Json);

impl Retries {
    #[must_use]
    pub fn pop(data: &mut Extra) -> Self {
        Self(
            data.shift_remove(MAX_RETRIES_KEY)
                .unwrap_or_else(|| Json::Number(serde_json::Number::from(DEFAULT_MAX_RETRIES))),
        )
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
}

// {"model": model, "prompt": prompt, **optional_params}: the overlay replaces model and prompt too
#[must_use]
pub fn build_data(
    model: Option<&str>,
    prompt: &str,
    optional_params: &Extra,
    cloudflare: bool,
) -> Extra {
    let mut data = Extra::new();
    let wire_model = match (cloudflare, model) {
        (false, Some(name)) => Json::Text(name.to_owned()),
        (false, None) | (true, _) => Json::Null,
    };
    data.insert("model".to_owned(), wire_model);
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
