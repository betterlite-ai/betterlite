use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use thiserror::Error;
use url::Url;

use super::client::VideoRequest;
use super::records::{Headers, Params};

#[derive(Debug, Error)]
pub(super) enum WireError {
    #[error("the request URL {url} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: http::header::InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the {name} query value is not a scalar")]
    QueryValue { name: String },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

pub(super) fn json_post(
    url: &str,
    headers: &Headers,
    data: &Params,
    timeout: Option<Duration>,
) -> Result<VideoRequest, WireError> {
    let mut request = base(url, Method::POST, headers)?;
    request.body = RequestBody::Json(serde_json::to_vec(data).map_err(WireError::Encode)?);
    request.timeout = timeout;
    Ok(request)
}

pub(super) fn query_get(
    url: &str,
    headers: &Headers,
    params: &Params,
) -> Result<VideoRequest, WireError> {
    let mut request = base(url, Method::GET, headers)?;
    request.query = query_pairs(params)?;
    Ok(request)
}

// The parts both sends share: the parsed URL, the wire headers, no query or body and no timeout
fn base(url: &str, method: Method, headers: &Headers) -> Result<VideoRequest, WireError> {
    Ok(VideoRequest {
        method,
        url: Url::parse(url).map_err(|source| WireError::Url {
            url: url.to_owned(),
            source,
        })?,
        query: Vec::new(),
        headers: wire_headers(headers)?,
        body: RequestBody::Empty,
        timeout: None,
    })
}

fn wire_headers(headers: &Headers) -> Result<HeaderMap, WireError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| WireError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| WireError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// httpx encodes a list as repeated keys and None as an empty value; a nested list or an object has no query form
fn query_pairs(params: &Params) -> Result<Vec<(String, String)>, WireError> {
    let mut pairs = Vec::new();
    for (name, value) in params {
        match value {
            Json::List(values) => {
                for item in values {
                    pairs.push((name.clone(), query_scalar(name, item)?));
                }
            }
            Json::Object(_) => return Err(WireError::QueryValue { name: name.clone() }),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                pairs.push((name.clone(), query_scalar(name, value)?));
            }
        }
    }
    Ok(pairs)
}

fn query_scalar(name: &str, value: &Json) -> Result<String, WireError> {
    match value {
        Json::Null => Ok(String::new()),
        Json::Bool(value) => Ok(value.to_string()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Text(text) => Ok(text.clone()),
        Json::List(_) | Json::Object(_) => Err(WireError::QueryValue {
            name: name.to_owned(),
        }),
    }
}
