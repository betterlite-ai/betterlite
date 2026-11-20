use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use thiserror::Error;
use url::Url;

use super::exchange::{EvalConnection, EvalExchange, EvalMethod};

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
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {name} query value is not a scalar")]
    QueryValue { name: String },
    #[error("the request body or query could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// The request one exchange puts on the wire; a call that names no timeout keeps the client's own read timeout, as httpx does
pub(super) fn outbound(
    connection: &EvalConnection<'_>,
    exchange: &EvalExchange<'_>,
) -> Result<OutboundRequest, WireError> {
    Ok(OutboundRequest {
        method: method(exchange.method),
        url: Url::parse(connection.url).map_err(|source| WireError::Url {
            url: connection.url.to_owned(),
            source,
        })?,
        query: query_pairs(exchange.query)?,
        headers: wire_headers(connection.headers)?,
        body: body(exchange.body)?,
        deadline: exchange
            .timeout
            .unwrap_or(connection.client.limits.read_timeout),
    })
}

const fn method(value: EvalMethod) -> Method {
    match value {
        EvalMethod::Get => Method::GET,
        EvalMethod::Post => Method::POST,
        EvalMethod::Delete => Method::DELETE,
    }
}

fn body(body: Option<&Object>) -> Result<RequestBody, WireError> {
    let Some(fields) = body else {
        return Ok(RequestBody::Empty);
    };
    let encoded = serde_json::to_vec(fields).map_err(WireError::Encode)?;
    Ok(RequestBody::Json(encoded))
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
fn query_pairs(query: Option<&Object>) -> Result<Vec<(String, String)>, WireError> {
    let Some(fields) = query else {
        return Ok(Vec::new());
    };
    let converted = serde_json::to_value(fields)
        .and_then(serde_json::from_value::<IndexMap<String, Json>>)
        .map_err(WireError::Encode)?;
    let mut pairs = Vec::with_capacity(converted.len());
    for (name, value) in &converted {
        match value {
            Json::List(items) => {
                for item in items {
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
        Json::Bool(flag) => Ok(flag.to_string()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Text(text) => Ok(text.clone()),
        Json::List(_) | Json::Object(_) => Err(WireError::QueryValue {
            name: name.to_owned(),
        }),
    }
}
