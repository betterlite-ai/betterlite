use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::merge_query;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use url::Url;

use super::super::config::{FileParams, FileResponse, Headers, MetadataConfig};
use super::super::error::{MetadataError, provider_error};
use super::{Attempt, Operation, facts};
use crate::api::vector_store_http_write::error::header_text;
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest, WriteTransport};

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

// Only the async delete and both listing sends pass the caller's timeout; the selected client's own default covers the rest
pub(super) const fn timeout_for(
    operation: Operation,
    mode: ClientKind,
    timeout: Option<Duration>,
) -> Option<Duration> {
    if operation.sends_timeout(mode) {
        timeout
    } else {
        None
    }
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
        MetadataError::Provider(provider_error(
            &facts::transport_facts(&error, attempt),
            config,
        ))
    })
}

// The delete client raises for a 4xx or 5xx with httpx's masked response, the way B2402's converter receives one
pub(super) fn delete_response<C>(
    config: &C,
    reply: &UpstreamResponse,
    url: &str,
) -> Result<FileResponse, MetadataError>
where
    C: MetadataConfig,
{
    if reply.status >= 400 {
        let facts = facts::status_facts(reply);
        return Err(MetadataError::Provider(provider_error(&facts, config)));
    }
    Ok(file_response(reply, url))
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
