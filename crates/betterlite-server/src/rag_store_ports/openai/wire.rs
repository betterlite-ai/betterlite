use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::str::from_utf8;

use betterlite_core::api::vector_store_http_write::config::WriteConfig;
use betterlite_core::api::vector_store_http_write::error::header_map;
use betterlite_core::rag::openai_ingestion::{AttachFile, CreateStore, PortError};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreChunkingStrategy, VectorStoreExpirationPolicy,
};
use betterlite_openai::vector_stores::files::FileCreateParams;
use betterlite_openai::vector_stores::store::CreateParams;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use http::{HeaderName, Method, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::rag_store_ports::exchange;

#[derive(Debug, thiserror::Error)]
pub(super) enum OpenAiPortError {
    #[error("the api_base {url:?} is not a URL")]
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
    Header {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the request body could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the chunking_strategy is not a chunking strategy")]
    Chunking {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file purpose {purpose:?} is not a purpose the file create accepts")]
    Purpose { purpose: String },
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
}

pub(super) fn port_error<Failure>(failure: Failure) -> PortError
where
    Failure: StdError + Send + Sync + 'static,
{
    PortError {
        message: failure.to_string(),
        source: Box::new(failure),
    }
}

pub(super) async fn post(
    client: &ProxiedClient,
    url: &str,
    headers: HeaderMap,
    body: RequestBody,
) -> Result<UpstreamResponse, PortError> {
    let target = Url::parse(url).map_err(|source| {
        port_error(OpenAiPortError::Url {
            url: url.to_owned(),
            source,
        })
    })?;
    exchange(
        client,
        OutboundRequest {
            method: Method::POST,
            url: target,
            query: Vec::new(),
            headers,
            body,
            deadline: client.limits.read_timeout,
        },
    )
    .await
    .map_err(|source| port_error(OpenAiPortError::Transport { source }))
}

pub(super) fn openai_headers(
    headers: BTreeMap<String, SecretString>,
) -> Result<HeaderMap, OpenAiPortError> {
    let mut mapped = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            OpenAiPortError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let mut header_value = HeaderValue::from_str(value.expose_secret())
            .map_err(|source| OpenAiPortError::Header { name, source })?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        mapped.insert(header_name, header_value);
    }
    Ok(mapped)
}

pub(super) fn store_params(request: &CreateStore<'_>) -> CreateParams {
    CreateParams {
        name: Some(request.name.to_owned()),
        file_ids: None,
        expires_after: request
            .expires_after
            .map(|expires| VectorStoreExpirationPolicy {
                anchor: Some(expires.anchor.to_owned()),
                days: Some(expires.days),
                extra: BTreeMap::new(),
            }),
        chunking_strategy: None,
        metadata: None,
    }
}

pub(super) fn file_params(request: &AttachFile<'_>) -> Result<FileCreateParams, OpenAiPortError> {
    let chunking_strategy = serde_json::from_str::<Option<VectorStoreChunkingStrategy>>(
        request.chunking_strategy.get(),
    )
    .map_err(|source| OpenAiPortError::Chunking { source })?;
    Ok(FileCreateParams {
        file_id: Some(request.file_id.to_owned()),
        attributes: None,
        chunking_strategy,
    })
}

pub(super) fn reply_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

pub(super) fn status_error(provider: &dyn WriteConfig, reply: &UpstreamResponse) -> PortError {
    let message = reply_text(&reply.body);
    port_error(provider.error_class(&message, reply.status, header_map(&reply.headers)))
}

pub(super) fn store_id(
    provider: &dyn WriteConfig,
    reply: &UpstreamResponse,
) -> Result<Option<String>, PortError> {
    let decoded = provider
        .transform_create_response(&http_response(reply))
        .map_err(port_error)?;
    let Json::Object(mut fields) = decoded else {
        return Ok(None);
    };
    match fields.shift_remove("id") {
        Some(Json::Text(id)) => Ok(Some(id)),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Ok(None)
        }
    }
}

fn http_response(reply: &UpstreamResponse) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(reply.body.clone());
    *response.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}
