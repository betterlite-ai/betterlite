use betterlite_http::user_url::httpx_url::HttpxUrl;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use super::client::RequestAuth;
use super::error::AzureSpeechError;
use crate::transcription::encoding::quote_plus;

// The SDK appends the route to a client base that ends in a slash, and every selected client hands one with it.
pub(super) fn compose_url(
    base: &str,
    deployment: Option<&str>,
    path: &str,
    version: Option<&str>,
    extra_query: &[(String, String)],
) -> Result<Url, AzureSpeechError> {
    let mut parsed = HttpxUrl::parse(base).map_err(AzureSpeechError::BaseUrl)?;
    let base_query = parsed.query.take();
    parsed.fragment = None;
    let prefix = match deployment {
        Some(name) if !parsed.path.contains("/deployments") => format!("deployments/{name}/"),
        Some(_) | None => String::new(),
    };
    let composed_text = format!("{}{prefix}{path}", parsed.href());
    let mut composed =
        HttpxUrl::parse(&composed_text).map_err(|source| AzureSpeechError::ComposedUrl {
            url: composed_text.clone(),
            source,
        })?;
    composed.query = merged_query(base_query.as_deref(), version, extra_query);
    Url::parse(&composed.href()).map_err(|source| AzureSpeechError::RequestUrl {
        url: composed.href(),
        source,
    })
}

// The client's own api-version comes first and the caller's extra query overrides it, as the SDK's params merge does.
fn merged_query(
    base_query: Option<&str>,
    api_version: Option<&str>,
    extra_query: &[(String, String)],
) -> Option<String> {
    let mut pairs: Vec<String> = base_query.map(split_pairs).unwrap_or_default();
    if let Some(version) = api_version {
        let name = quote_plus("api-version");
        upsert(&mut pairs, format!("{name}={}", quote_plus(version)), &name);
    }
    for (key, value) in extra_query {
        let name = quote_plus(key);
        let encoded = format!("{name}={}", quote_plus(value));
        upsert(&mut pairs, encoded, &name);
    }
    if pairs.is_empty() {
        return None;
    }
    Some(pairs.join("&"))
}

fn split_pairs(query: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(str::to_owned)
        .collect()
}

fn upsert(pairs: &mut Vec<String>, encoded: String, key: &str) {
    match pairs
        .iter_mut()
        .find(|pair| pair.split('=').next() == Some(key))
    {
        Some(pair) => *pair = encoded,
        None => pairs.push(encoded),
    }
}

// The speech post's headers: the SDK's Accept, the caller's extra headers, then the credential it would add.
pub(super) fn binary_headers(
    extra: &[(String, String)],
    auth: Option<&RequestAuth>,
) -> Result<HeaderMap, AzureSpeechError> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/octet-stream"));
    for (name, value) in extra {
        headers.insert(header_name(name)?, header_value(name, value)?);
    }
    match auth {
        Some(RequestAuth::ApiKey(key)) => {
            let name = HeaderName::from_static("api-key");
            if !headers.contains_key(&name) {
                headers.insert(name, sensitive_value("api-key", key.expose_secret())?);
            }
        }
        Some(RequestAuth::Bearer(token)) => {
            let name = AUTHORIZATION;
            if !headers.contains_key(&name) {
                let value = format!("Bearer {}", token.expose_secret());
                headers.insert(name, sensitive_value("authorization", &value)?);
            }
        }
        None => {}
    }
    Ok(headers)
}

pub(super) fn header_name(name: &str) -> Result<HeaderName, AzureSpeechError> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|source| AzureSpeechError::HeaderName {
        name: name.to_owned(),
        source,
    })
}

pub(super) fn header_value(name: &str, value: &str) -> Result<HeaderValue, AzureSpeechError> {
    HeaderValue::try_from(value).map_err(|source| AzureSpeechError::HeaderValue {
        name: name.to_owned(),
        source,
    })
}

pub(super) fn sensitive_value(name: &str, value: &str) -> Result<HeaderValue, AzureSpeechError> {
    let mut parsed = header_value(name, value)?;
    parsed.set_sensitive(true);
    Ok(parsed)
}
