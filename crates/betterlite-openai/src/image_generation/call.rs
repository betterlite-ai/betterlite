use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ImageGenerationSource;
use crate::openai_base_config::Headers;

const DEFAULT_MAX_RETRIES: i64 = 2;

pub struct ImageGenerationCall {
    pub model: String,
    pub prompt: String,
    pub optional_params: IndexMap<String, Json>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub organization: Option<String>,
    pub headers: Option<Headers>,
    pub timeout: Duration,
    pub delegate_async: bool,
    pub now_seconds: i64,
}

#[derive(Debug)]
pub struct PreparedImageCall {
    pub prompt: String,
    pub api_base: String,
    pub data: IndexMap<String, Json>,
    pub wire_data: IndexMap<String, Json>,
    pub timeout: Duration,
    pub max_retries: i64,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub async_mode: bool,
    pub now_seconds: i64,
}

#[derive(Debug)]
pub struct PrepareFailure {
    pub data: IndexMap<String, Json>,
    pub source: ImageGenerationSource,
}

// the data dict merges model and prompt under the optional params, then takes max_retries out of the body
pub fn prepare_image_call(call: ImageGenerationCall) -> Result<PreparedImageCall, PrepareFailure> {
    let base = call
        .api_base
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let mut data = IndexMap::new();
    data.insert(String::from("model"), Json::Text(call.model));
    data.insert(String::from("prompt"), Json::Text(call.prompt.clone()));
    data.extend(call.optional_params);
    let max_retries = match take_max_retries(&mut data) {
        Ok(value) => value,
        Err(source) => return Err(PrepareFailure { data, source }),
    };
    let wire_data = wire_body(&data, call.headers);
    Ok(PreparedImageCall {
        prompt: call.prompt,
        api_base: base,
        data,
        wire_data,
        timeout: call.timeout,
        max_retries,
        api_key: call.api_key,
        organization: call.organization,
        async_mode: call.delegate_async,
        now_seconds: call.now_seconds,
    })
}

fn take_max_retries(data: &mut IndexMap<String, Json>) -> Result<i64, ImageGenerationSource> {
    match data.shift_remove("max_retries") {
        None => Ok(DEFAULT_MAX_RETRIES),
        Some(Json::Number(number)) => number.as_i64().ok_or(ImageGenerationSource::MaxRetries),
        Some(_) => Err(ImageGenerationSource::MaxRetries),
    }
}

pub(super) fn authorization_headers(api_key: &SecretString) -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("Authorization"),
        Json::Text(format!("Bearer {}", api_key.expose_secret())),
    )])
}

// a nonempty headers argument replaces the caller's extra_headers in the body alone
fn wire_body(data: &IndexMap<String, Json>, headers: Option<Headers>) -> IndexMap<String, Json> {
    let mut body = data.clone();
    if let Some(explicit) = headers.filter(|explicit| !explicit.is_empty()) {
        let fields = explicit
            .into_iter()
            .map(|(name, value)| (name, Json::Text(value)))
            .collect();
        body.insert(String::from("extra_headers"), Json::Object(fields));
    }
    body
}
