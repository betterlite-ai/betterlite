use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::merge_query;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use serde_json::Number;
use url::Url;

use super::super::config::{FileParams, FileResponse, Headers, MetadataConfig};
use super::super::error::{MetadataError, provider_error};
use super::{Attempt, ClientKind, Operation};
use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};
use crate::api::vector_store_http_write::service::{WriteRequest, WriteTransport};

pub(super) fn query_pairs(query: &FileParams) -> Result<Vec<(String, String)>, MetadataError> {
    let mut pairs = Vec::with_capacity(query.len());
    for (name, value) in query {
        let Some(text) = value.as_str() else {
            return Err(MetadataError::QueryValue { name: name.clone() });
        };
        pairs.push((name.clone(), text.to_owned()));
    }
    Ok(pairs)
}

pub(super) fn request_url(base: &str, query: &[(String, String)]) -> Result<Url, MetadataError> {
    let mut url = Url::parse(base).map_err(|source| MetadataError::RequestUrl {
        url: base.to_owned(),
        source,
    })?;
    merge_query(&mut url, query);
    Ok(url)
}

pub(super) fn wire_headers(headers: &Headers) -> Result<HttpHeaders, MetadataError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            MetadataError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| MetadataError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

pub(super) async fn send_once<C, Client>(
    config: &C,
    client: &Client,
    request: WriteRequest,
    attempt: Attempt,
) -> Result<UpstreamResponse, MetadataError>
where
    C: MetadataConfig,
    Client: WriteTransport,
{
    client.send(request).await.map_err(|error| {
        MetadataError::Provider(provider_error(&transport_facts(&error, attempt), config))
    })
}

pub(super) fn file_response(reply: &UpstreamResponse, url: &str) -> FileResponse {
    FileResponse {
        status_code: reply.status,
        headers: reply
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
            .collect(),
        body: reply.body.clone(),
        url: Some(url.to_owned()),
    }
}

// httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let mut headers: HeaderMap = reply
        .headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect();
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert("content-length".to_owned(), Json::Text(length));
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

// The sync delete client catches httpx.TimeoutException and raises litellm.Timeout(408); every other send lets the raw error out
pub(super) fn transport_facts(error: &TransportError, attempt: Attempt) -> FailureFacts {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) && matches!(
        (attempt.operation, attempt.mode),
        (Operation::Delete, ClientKind::Sync)
    );
    if timed_out {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some("litellm.Timeout: Connection timed out after None seconds.".to_owned()),
            headers: Some(HeaderMap::new()),
            response: None,
            display: String::new(),
        };
    }
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    }
}
