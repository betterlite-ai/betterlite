use std::collections::BTreeMap;
use std::io::{Error, Read as _};
use std::str::{self, Utf8Error};
use std::time::Duration;

use crate::OracleError;
use crate::case::{Request, RequestBody};
use crate::multipart_oracle::request::request_body;
use reqwest::blocking::{Client, ClientBuilder, RequestBuilder};
use reqwest::redirect::Policy;
use serde_json::Value;

#[path = "http/native_feature_eligibility/native_feature_eligibility.rs"]
pub mod native_feature_eligibility;
pub mod postgres;

pub(crate) const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[expect(
    clippy::disallowed_methods,
    reason = "The CLI owns its HTTP client and configures timeouts and local-only transport here."
)]
pub(crate) fn client() -> Result<Client, OracleError> {
    let client = ClientBuilder::from(reqwest::Client::builder())
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(OracleError::HttpRequest)?;
    Ok(client)
}

pub(crate) fn send(
    client: &Client,
    base: &str,
    request: &Request,
) -> Result<Response, OracleError> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(Error::other)
        .map_err(OracleError::HttpIo)?;
    let mut outgoing = client.request(method, format!("{base}{}", request.path));
    for (name, value) in &request.headers {
        outgoing = outgoing.header(name, value);
    }
    let body = request_body(&request.body, request.body_kind).map_err(OracleError::Multipart)?;
    let incoming = with_body(outgoing, body)
        .send()
        .map_err(OracleError::HttpRequest)?;
    let status = incoming.status().as_u16();
    let headers = incoming
        .headers()
        .iter()
        .map(|(name, value)| {
            Ok((
                name.to_string(),
                str::from_utf8(value.as_bytes())?.to_owned(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Utf8Error>>()
        .map_err(OracleError::HttpText)?;
    let mut bytes = Vec::new();
    incoming
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(OracleError::HttpIo)?;
    if bytes.len() > MAX_BODY {
        return Err(OracleError::HttpTooLarge);
    }
    Ok(Response {
        status,
        body: parse_body(&headers, &bytes)?,
        headers,
    })
}

pub(crate) fn ready(client: &Client, base: &str) -> Result<bool, OracleError> {
    let response = client
        .get(format!("{base}/health/liveliness"))
        .send()
        .map_err(OracleError::HttpRequest)?;
    Ok(response.status() == reqwest::StatusCode::OK)
}

fn with_body(outgoing: RequestBuilder, body: RequestBody) -> RequestBuilder {
    match body {
        RequestBody::Empty => outgoing,
        RequestBody::Json(bytes) => outgoing.body(bytes),
        RequestBody::Raw {
            content_type,
            bytes,
        } => outgoing.header("content-type", content_type).body(bytes),
    }
}

pub(crate) fn parse_body(
    headers: &BTreeMap<String, String>,
    bytes: &[u8],
) -> Result<Value, OracleError> {
    if headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/octet-stream"))
    {
        return Ok(Value::Array(
            bytes.iter().map(|byte| Value::from(*byte)).collect(),
        ));
    }
    let text = str::from_utf8(bytes).map_err(OracleError::HttpText)?;
    if headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        let events = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|line| *line != "[DONE]")
            .map(serde_json::from_str)
            .collect::<Result<Vec<Value>, serde_json::Error>>()
            .map_err(OracleError::HttpBody)?;
        return Ok(Value::Array(events));
    }
    Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
}
