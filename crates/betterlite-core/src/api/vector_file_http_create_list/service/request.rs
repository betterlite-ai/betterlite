use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::config::FilesConfig;
use super::super::error::FileTransportError;
use super::super::payload::query_pairs;
use super::{FileRequest, FileTarget, Operation};

// B527's validate_environment rewrites Authorization and Content-Type in the map it is handed, so the caller's entries go in first
pub(super) fn validated_headers<C>(config: &C, target: &FileTarget<'_>) -> OpenAiHeaders
where
    C: FilesConfig,
{
    let caller_headers = target
        .extra_headers
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    config.validate_environment(caller_headers, target.params)
}

pub(super) fn endpoint<C>(
    config: &C,
    target: &FileTarget<'_>,
) -> Result<(String, Url), FileTransportError>
where
    C: FilesConfig,
{
    let text = config
        .complete_url(target.params, target.vector_store_id)
        .map_err(FileTransportError::Url)?;
    let url = Url::parse(&text).map_err(|source| FileTransportError::RequestUrl {
        url: text.clone(),
        source,
    })?;
    Ok((text, url))
}

// The create handlers pass the timeout in both modes; the list handlers accept one and never hand it to the client
pub(super) const fn call_timeout(
    operation: Operation,
    timeout: Option<Duration>,
) -> Option<Duration> {
    match operation {
        Operation::Create => timeout,
        Operation::List => None,
    }
}

pub(super) fn wire_headers(headers: &OpenAiHeaders) -> Result<HttpHeaders, FileTransportError> {
    headers
        .iter()
        .map(|(name, value)| {
            let header = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
                FileTransportError::HeaderName {
                    name: name.clone(),
                    source,
                }
            })?;
            let text = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
                FileTransportError::HeaderValue {
                    name: name.clone(),
                    source,
                }
            })?;
            Ok((header, text))
        })
        .collect()
}

pub(super) fn wire_request(
    operation: Operation,
    url: Url,
    fields: &IndexMap<String, Json>,
    headers: HttpHeaders,
    timeout: Option<Duration>,
) -> Result<FileRequest, FileTransportError> {
    let (query, body) = match operation {
        Operation::Create => (
            Vec::new(),
            RequestBody::Json(serde_json::to_vec(fields).map_err(FileTransportError::Encode)?),
        ),
        Operation::List => (query_pairs(fields)?, RequestBody::Empty),
    };
    Ok(FileRequest {
        method: operation.http_method(),
        url,
        query,
        headers,
        body,
        timeout,
    })
}
