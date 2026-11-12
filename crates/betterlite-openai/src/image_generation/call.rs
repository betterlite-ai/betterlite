use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use betterlite_models::decisions::Json;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::error::ImageGenerationSource;
use crate::openai_base_config::Headers;

const DEFAULT_MAX_RETRIES: i64 = 2;
const IMAGES_PATH: &str = "/images/generations";

pub struct ImageGenerationCall {
    pub model: String,
    pub prompt: String,
    pub optional_params: IndexMap<String, Json>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub organization: Option<String>,
    pub headers: Option<Headers>,
    pub timeout: Option<Duration>,
    pub delegate_async: bool,
    pub now_seconds: i64,
}

pub struct PreparedImageCall {
    pub prompt: String,
    pub api_base: String,
    pub url: Url,
    pub data: IndexMap<String, Json>,
    pub wire_data: IndexMap<String, Json>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
    pub max_retries: i64,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub async_mode: bool,
    pub now_seconds: i64,
}

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
    let (url, wire_headers) = match wire_call(&base, call.api_key.as_ref()) {
        Ok(found) => found,
        Err(source) => return Err(PrepareFailure { data, source }),
    };
    let wire_data = wire_body(&data, call.headers);
    Ok(PreparedImageCall {
        prompt: call.prompt,
        api_base: base,
        url,
        data,
        wire_data,
        headers: wire_headers,
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

pub(super) fn authorization_headers(api_key: Option<&SecretString>) -> IndexMap<String, Json> {
    let key = api_key.map_or("None", |key| key.expose_secret().as_str());
    IndexMap::from([(
        String::from("Authorization"),
        Json::Text(format!("Bearer {key}")),
    )])
}

fn wire_call(
    base: &str,
    api_key: Option<&SecretString>,
) -> Result<(Url, HeaderMap), ImageGenerationSource> {
    let url = images_url(base)?;
    let headers = request_headers(api_key)?;
    Ok((url, headers))
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

fn request_headers(api_key: Option<&SecretString>) -> Result<HeaderMap, ImageGenerationSource> {
    let key = api_key.map_or("None", |key| key.expose_secret().as_str());
    let value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|source| {
        ImageGenerationSource::RequestHeader {
            name: String::from("Authorization"),
            source,
        }
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn images_url(api_base: &str) -> Result<Url, ImageGenerationSource> {
    let mut url = Url::parse(api_base).map_err(ImageGenerationSource::Url)?;
    let path = format!("{}{IMAGES_PATH}", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}
