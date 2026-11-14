use std::str::FromStr as _;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute};
use betterlite_models::controls::json_value::JsonValue;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::{Params, PreparedRequest, RequestFailure, RequestHeaders};
use crate::proxy::outbound::merge_query;

pub(crate) mod response;

// the GET target: the transform's query params join the URL's own, and a key already in the URL wins
pub(crate) fn request_url(prepared: &PreparedRequest) -> Result<Url, RequestFailure> {
    let mut url = Url::parse(&prepared.url).map_err(RequestFailure::Url)?;
    let pairs = query_pairs(&prepared.params)?;
    merge_query(&mut url, &pairs);
    Ok(url)
}

// httpx renders a null query value as the empty string, and a list as one pair per item
pub(crate) fn query_pairs(params: &Params) -> Result<Vec<(String, String)>, RequestFailure> {
    let mut pairs = Vec::with_capacity(params.len());
    for (name, value) in params {
        match value {
            JsonValue::Null => pairs.push((name.clone(), String::new())),
            JsonValue::Text(text) => pairs.push((name.clone(), text.clone())),
            JsonValue::Bool(flag) => pairs.push((name.clone(), flag.to_string())),
            JsonValue::Number(number) => pairs.push((name.clone(), number.to_string())),
            JsonValue::Array(items) => repeated_pairs(name, items, &mut pairs)?,
            JsonValue::Object(_) => {
                return Err(RequestFailure::QueryValue {
                    field: name.clone(),
                });
            }
        }
    }
    Ok(pairs)
}

fn repeated_pairs(
    name: &str,
    items: &[JsonValue],
    pairs: &mut Vec<(String, String)>,
) -> Result<(), RequestFailure> {
    for item in items {
        let text = match item {
            JsonValue::Null => String::new(),
            JsonValue::Text(text) => text.clone(),
            JsonValue::Bool(flag) => flag.to_string(),
            JsonValue::Number(number) => number.to_string(),
            JsonValue::Array(_) | JsonValue::Object(_) => {
                return Err(RequestFailure::QueryValue {
                    field: name.to_owned(),
                });
            }
        };
        pairs.push((name.to_owned(), text));
    }
    Ok(())
}

// a proxy setting betterlite cannot use is refused, so the provider key never goes out directly instead
pub(crate) fn refuse_unusable_proxy(policy: &ProxyPolicy, url: &Url) -> Result<(), RequestFailure> {
    match policy.route(url) {
        Some(ProxyRoute::Unsupported { scheme }) => Err(RequestFailure::UnsupportedProxyScheme {
            scheme: scheme.clone(),
        }),
        Some(ProxyRoute::Invalid { name }) => {
            Err(RequestFailure::InvalidProxyUrl { name: name.clone() })
        }
        Some(ProxyRoute::Forward(_)) | None => Ok(()),
    }
}

pub(crate) fn request_headers(headers: &RequestHeaders) -> Result<HeaderMap, RequestFailure> {
    headers
        .iter()
        .map(|(name, value)| header_entry(name, value))
        .collect::<Result<HeaderMap, RequestFailure>>()
}

fn header_entry(
    name: &str,
    value: &SecretString,
) -> Result<(HeaderName, HeaderValue), RequestFailure> {
    let header_value =
        HeaderValue::from_str(value.expose_secret()).map_err(RequestFailure::HeaderValue)?;
    let header_name = HeaderName::from_str(name).map_err(RequestFailure::HeaderName)?;
    Ok((header_name, header_value))
}
